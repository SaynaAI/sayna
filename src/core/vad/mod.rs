//! Voice Activity Detection (VAD) module using Silero VAD.
//!
//! This module provides voice activity detection capabilities for STT providers,
//! enabling them to detect speech segments in audio streams.
//!
//! The module is feature-gated behind `stt-vad`. When disabled, a no-op stub
//! implementation is provided.

#[cfg(feature = "stt-vad")]
pub mod assets;
pub mod config;
#[cfg(feature = "stt-vad")]
pub mod detector;
#[cfg(feature = "stt-vad")]
pub mod model_manager;
pub mod silence_tracker;

#[cfg(not(feature = "stt-vad"))]
mod stub;

pub use config::{GraphOptimizationLevel, SileroVADConfig, VADSampleRate, VADSilenceConfig};
pub use silence_tracker::{SilenceTracker, SilenceTrackerConfig, VADEvent};

#[cfg(feature = "stt-vad")]
pub use detector::{SileroVAD, SileroVADBuilder};

#[cfg(not(feature = "stt-vad"))]
pub use stub::{SileroVAD, create_silero_vad};

#[cfg(feature = "stt-vad")]
use anyhow::Result;
#[cfg(feature = "stt-vad")]
use std::path::Path;

/// Create a new Silero VAD detector instance.
///
/// If `model_path` is provided, loads the model from that path.
/// Otherwise, uses the default model URL or cached model.
#[cfg(feature = "stt-vad")]
pub async fn create_silero_vad(model_path: Option<&Path>) -> Result<SileroVAD> {
    SileroVAD::new(model_path).await
}
