//! Initialization helpers for preparing runtime assets before starting the
//! Sayna server.
//!
//! This module hosts the logic that powers the `sayna init` CLI command. The
//! command downloads and caches the turn detection model and tokenizer so that
//! regular server startups do not have to perform network fetches.
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

#[cfg(any(feature = "turn-detect", feature = "kokoro-tts"))]
use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;

#[cfg(any(feature = "turn-detect", feature = "kokoro-tts"))]
use crate::config::ServerConfig;
#[cfg(feature = "kokoro-tts")]
use crate::core::tts::kokoro::assets as kokoro_assets;
#[cfg(feature = "turn-detect")]
use crate::core::turn_detect::{TurnDetectorConfig, assets};

/// Download and prepare all assets required for runtime execution.
#[cfg(any(feature = "turn-detect", feature = "kokoro-tts"))]
pub async fn run() -> Result<()> {
    let config = ServerConfig::from_env().map_err(|e| anyhow!(e.to_string()))?;
    let cache_path = config
        .cache_path
        .as_ref()
        .context("CACHE_PATH environment variable must be set to run `sayna init`")?
        .clone();

    #[cfg(feature = "turn-detect")]
    {
        let turn_config = TurnDetectorConfig {
            cache_path: Some(cache_path.clone()),
            ..Default::default()
        };

        tracing::info!(
            "Preparing turn detector assets using cache path: {:?}",
            cache_path
        );
        assets::download_assets(&turn_config).await?;
        tracing::info!("Turn detector assets downloaded successfully");
    }

    #[cfg(feature = "kokoro-tts")]
    {
        let kokoro_cache = cache_path.join("kokoro");
        let kokoro_config = kokoro_assets::KokoroAssetConfig {
            cache_path: kokoro_cache.clone(),
        };

        tracing::info!(
            "Preparing Kokoro TTS assets from HuggingFace using cache path: {:?}",
            kokoro_cache
        );
        kokoro_assets::download_assets(&kokoro_config).await?;
        tracing::info!("Kokoro TTS assets downloaded successfully");
    }

    Ok(())
}

#[cfg(not(any(feature = "turn-detect", feature = "kokoro-tts")))]
pub async fn run() -> Result<()> {
    Err(anyhow!(
        "`sayna init` requires the `turn-detect` or `kokoro-tts` feature. \
         Rebuild with `--features turn-detect` or `--features kokoro-tts` to download assets."
    ))
}
