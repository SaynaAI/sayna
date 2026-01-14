//! Initialization helpers for preparing runtime assets before starting the
//! Sayna server.
//!
//! This module hosts the logic that powers the `sayna init` CLI command. The
//! command downloads and caches models required by optional features:
//!
//! - **turn-detect**: Turn detection model and tokenizer
//! - **stt-vad**: Silero-VAD model for voice activity detection
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

#[cfg(any(feature = "turn-detect", feature = "stt-vad"))]
use crate::config::ServerConfig;
#[cfg(feature = "turn-detect")]
use crate::core::turn_detect::{TurnDetectorConfig, assets as turn_assets};
#[cfg(feature = "stt-vad")]
use crate::core::vad::{SileroVADConfig, assets as vad_assets};
#[cfg(any(feature = "turn-detect", feature = "stt-vad"))]
use anyhow::Context;

/// Download and prepare all assets required for runtime execution.
///
/// Downloads models for enabled features:
/// - Turn detection model (if `turn-detect` feature is enabled)
/// - Silero-VAD model (if `stt-vad` feature is enabled)
#[cfg(any(feature = "turn-detect", feature = "stt-vad"))]
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
    #[cfg(feature = "turn-detect")]
    {
        tracing::info!("Downloading turn detection model...");
        let turn_config = TurnDetectorConfig {
            cache_path: Some(cache_path.clone()),
            ..Default::default()
        };
        match turn_assets::download_assets(&turn_config).await {
            Ok(_) => tracing::info!("Turn detection model downloaded successfully."),
            Err(e) => {
                tracing::error!("Failed to download turn detection model: {}", e);
                tracing::error!(
                    "You can manually download from: https://huggingface.co/livekit/turn-detector/resolve/main/model_quantized.onnx"
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

    // Verify downloaded assets
    verify_assets(&cache_path).await?;

    tracing::info!("Initialization complete!");
    Ok(())
}

/// Verify that all required assets are present.
#[cfg(any(feature = "turn-detect", feature = "stt-vad"))]
async fn verify_assets(cache_path: &std::path::Path) -> Result<()> {
    tracing::info!("Verifying downloaded assets...");

    #[cfg(feature = "turn-detect")]
    {
        let model_path = cache_path.join("turn_detect/model_quantized.onnx");
        let tokenizer_path = cache_path.join("turn_detect/tokenizer.json");
        if !model_path.exists() {
            anyhow::bail!(
                "Turn detection model missing at {:?}. Download may have failed.",
                model_path
            );
        }
        if !tokenizer_path.exists() {
            anyhow::bail!(
                "Turn detection tokenizer missing at {:?}. Download may have failed.",
                tokenizer_path
            );
        }
        tracing::info!("  Turn detection: OK");
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

    Ok(())
}

#[cfg(not(any(feature = "turn-detect", feature = "stt-vad")))]
pub async fn run() -> Result<()> {
    Err(anyhow!(
        "`sayna init` requires at least one of the following features:\n\
         - `turn-detect`: Download turn detection model\n\
         - `stt-vad`: Download Silero-VAD model\n\n\
         Rebuild with the desired features, for example:\n\
         cargo build --features turn-detect,stt-vad"
    ))
}
