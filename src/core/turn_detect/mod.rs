#[cfg(feature = "stt-vad")]
pub mod assets;
pub mod config;
#[cfg(feature = "stt-vad")]
pub mod detector;
#[cfg(feature = "stt-vad")]
pub mod model_manager;
#[cfg(feature = "stt-vad")]
pub mod tokenizer;

#[cfg(not(feature = "stt-vad"))]
mod stub;

pub use config::TurnDetectorConfig;

#[cfg(feature = "stt-vad")]
pub use detector::{TurnDetector, TurnDetectorBuilder};

#[cfg(not(feature = "stt-vad"))]
pub use stub::{TurnDetector, TurnDetectorBuilder, create_turn_detector};

#[cfg(feature = "stt-vad")]
use anyhow::Result;
#[cfg(feature = "stt-vad")]
use std::path::Path;

#[cfg(feature = "stt-vad")]
pub async fn create_turn_detector(model_path: Option<&Path>) -> Result<TurnDetector> {
    TurnDetector::new(model_path).await
}
