//! Initialization helpers for preparing runtime assets before starting the
//! Sayna server.
//!
//! This module hosts the logic that powers the `sayna init` CLI command. The
//! command downloads and caches models required by optional features:
//!
//! - **stt-vad**: Smart-turn model and Silero-VAD model for voice activity
//!   detection with integrated turn detection
//! - **noise-filter**: DeepFilterNet ONNX models for real-time noise suppression
//!
//! Typical usage from the CLI:
//!
//! ```text
//! $ CACHE_PATH=/app/cache sayna init
//! ```
//!
//! If you prefer to invoke the initialization routine programmatically, call
//! [`run`] inside an async context:
//!
//! ```rust,no_run
//! use sayna::init;
//!
//! let runtime = tokio::runtime::Runtime::new().unwrap();
//! runtime.block_on(async {
//!     init::run().await.expect("failed to download assets");
//! });
//! ```

use anyhow::Result;
use anyhow::anyhow;

#[cfg(any(feature = "stt-vad", feature = "noise-filter"))]
use crate::config::ServerConfig;
#[cfg(any(feature = "stt-vad", feature = "noise-filter"))]
use anyhow::Context;

#[cfg(feature = "stt-vad")]
use crate::core::turn_detect::{MODEL_FILENAME, TurnDetectorConfig, assets as turn_assets};
#[cfg(feature = "stt-vad")]
use crate::core::vad::{SileroVADConfig, assets as vad_assets};

#[cfg(feature = "noise-filter")]
use crate::core::noise_filter::{NoiseFilterConfig, assets as noise_filter_assets};

/// Download and prepare all assets required for runtime execution.
///
/// Downloads models for enabled features:
/// - Turn detection model (if `stt-vad` feature is enabled)
/// - Silero-VAD model (if `stt-vad` feature is enabled)
/// - DeepFilterNet models (if `noise-filter` feature is enabled)
#[cfg(any(feature = "stt-vad", feature = "noise-filter"))]
pub async fn run() -> Result<()> {
    let config = ServerConfig::from_env().map_err(|e| anyhow!(e.to_string()))?;
    let cache_path = config
        .cache_path
        .as_ref()
        .context("CACHE_PATH environment variable must be set to run `sayna init`")?
        .clone();

    tracing::info!("Initializing Sayna...");
    tracing::info!("Cache path: {:?}", cache_path);

    // Download turn detection assets
    #[cfg(feature = "stt-vad")]
    {
        tracing::info!("Downloading turn detection model...");
        let turn_config = TurnDetectorConfig {
            cache_path: Some(cache_path.clone()),
            ..Default::default()
        };
        match turn_assets::download_assets(&turn_config).await {
            Ok(_) => tracing::info!("Smart-turn model downloaded successfully."),
            Err(e) => {
                tracing::error!("Failed to download smart-turn model: {}", e);
                tracing::error!(
                    "You can manually download from: https://huggingface.co/pipecat-ai/smart-turn-v3/resolve/main/{}",
                    MODEL_FILENAME
                );
                tracing::error!("And place it at: {:?}", cache_path.join("turn_detect"));
                return Err(e);
            }
        }
    }

    // Download VAD assets
    #[cfg(feature = "stt-vad")]
    {
        tracing::info!("Downloading Silero-VAD model...");
        let vad_config = SileroVADConfig {
            cache_path: Some(cache_path.clone()),
            ..Default::default()
        };
        match vad_assets::download_assets(&vad_config).await {
            Ok(_) => tracing::info!("Silero-VAD model downloaded successfully."),
            Err(e) => {
                tracing::error!("Failed to download Silero-VAD model: {}", e);
                if let Some(url) = vad_config.model_url.as_ref() {
                    tracing::error!("You can manually download from: {}", url);
                }
                tracing::error!(
                    "And place it at: {:?}",
                    cache_path.join("vad/silero_vad.onnx")
                );
                return Err(e);
            }
        }
    }

    // Download noise filter assets
    #[cfg(feature = "noise-filter")]
    {
        tracing::info!("Downloading DeepFilterNet models...");
        let noise_filter_config = NoiseFilterConfig {
            cache_path: Some(cache_path.clone()),
            ..Default::default()
        };
        match noise_filter_assets::download_assets(&noise_filter_config).await {
            Ok(_) => tracing::info!("DeepFilterNet models downloaded successfully."),
            Err(e) => {
                tracing::error!("Failed to download DeepFilterNet models: {}", e);
                tracing::error!(
                    "You can manually download from: https://github.com/Rikorose/DeepFilterNet/raw/main/models/DeepFilterNet3_ll_onnx.tar.gz"
                );
                tracing::error!(
                    "Extract and place files at: {:?}",
                    cache_path.join("noise_filter")
                );
                return Err(e);
            }
        }
    }

    // Verify downloaded assets
    verify_assets(&cache_path).await?;

    tracing::info!("Initialization complete!");
    Ok(())
}

/// Verify that all required assets are present.
#[cfg(any(feature = "stt-vad", feature = "noise-filter"))]
async fn verify_assets(cache_path: &std::path::Path) -> Result<()> {
    tracing::info!("Verifying downloaded assets...");

    #[cfg(feature = "stt-vad")]
    {
        let model_path = cache_path.join(format!("turn_detect/{}", MODEL_FILENAME));
        if !model_path.exists() {
            anyhow::bail!(
                "Smart-turn model missing at {:?}. Download may have failed.",
                model_path
            );
        }
        tracing::info!("  Smart-turn: OK");
    }

    #[cfg(feature = "stt-vad")]
    {
        let model_path = cache_path.join("vad/silero_vad.onnx");
        if !model_path.exists() {
            anyhow::bail!(
                "Silero-VAD model missing at {:?}. Download may have failed.",
                model_path
            );
        }
        tracing::info!("  Silero-VAD: OK");
    }

    #[cfg(feature = "noise-filter")]
    {
        use crate::core::noise_filter::config::{
            CONFIG_FILENAME, DF_DEC_MODEL_FILENAME, ENC_MODEL_FILENAME, ERB_DEC_MODEL_FILENAME,
        };

        let noise_filter_dir = cache_path.join("noise_filter");
        for filename in [
            ENC_MODEL_FILENAME,
            ERB_DEC_MODEL_FILENAME,
            DF_DEC_MODEL_FILENAME,
            CONFIG_FILENAME,
        ] {
            let model_path = noise_filter_dir.join(filename);
            if !model_path.exists() {
                anyhow::bail!(
                    "DeepFilterNet {} missing at {:?}. Download may have failed.",
                    filename,
                    model_path
                );
            }
        }
        tracing::info!("  DeepFilterNet: OK");
    }

    Ok(())
}

#[cfg(not(any(feature = "stt-vad", feature = "noise-filter")))]
pub async fn run() -> Result<()> {
    Err(anyhow!(
        "`sayna init` requires at least one of these features:\n\
         - `stt-vad`: Download turn detection and Silero-VAD models\n\
         - `noise-filter`: Download DeepFilterNet noise suppression models\n\n\
         Rebuild with a feature enabled, for example:\n\
         cargo build --features stt-vad\n\
         cargo build --features noise-filter"
    ))
}
