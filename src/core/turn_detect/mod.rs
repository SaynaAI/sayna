//! Smart-turn detection module for voice activity detection.
//!
//! This module provides audio-based turn detection using the smart-turn v3 model.
//! It processes raw audio samples (i16 PCM at 16kHz) through mel spectrogram
//! extraction and returns the probability that a speaker has finished their turn.
//!
//! # Feature Flags
//!
//! - `stt-vad`: Enables the full smart-turn implementation with ONNX inference.
//!   When disabled, a no-op stub is used that always returns false.
//!
//! # Example
//!
//! ```ignore
//! use sayna::core::turn_detect::{TurnDetector, TurnDetectorBuilder};
//!
//! // Create detector with default config
//! let detector = TurnDetector::new(None).await?;
//!
//! // Or use builder for custom config
//! let detector = TurnDetectorBuilder::new()
//!     .threshold(0.9)
//!     .num_threads(2)
//!     .build()
//!     .await?;
//!
//! // Check if turn is complete from audio samples
//! let audio_samples: &[i16] = &[/* 16kHz PCM samples */];
//! let is_complete = detector.is_turn_complete(audio_samples).await?;
//! ```

#[cfg(feature = "stt-vad")]
pub mod assets;
pub mod config;
#[cfg(feature = "stt-vad")]
pub mod detector;
#[cfg(feature = "stt-vad")]
pub mod feature_extractor;
#[cfg(feature = "stt-vad")]
pub mod model_manager;

#[cfg(not(feature = "stt-vad"))]
mod stub;

pub use config::TurnDetectorConfig;

#[cfg(feature = "stt-vad")]
pub use config::MODEL_FILENAME;

#[cfg(feature = "stt-vad")]
pub use detector::{TurnDetector, TurnDetectorBuilder};

#[cfg(feature = "stt-vad")]
pub use feature_extractor::FeatureExtractor;

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
