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

#[cfg(any(
    feature = "turn-detect",
    feature = "kokoro-tts",
    feature = "whisper-stt"
))]
use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;

#[cfg(any(
    feature = "turn-detect",
    feature = "kokoro-tts",
    feature = "whisper-stt"
))]
use crate::config::ServerConfig;
#[cfg(feature = "whisper-stt")]
use crate::core::stt::whisper::{
    DEFAULT_MODEL as WHISPER_DEFAULT_MODEL, WhisperAssetConfig,
    download_assets as whisper_download_assets,
};
#[cfg(feature = "kokoro-tts")]
use crate::core::tts::kokoro::assets as kokoro_assets;
#[cfg(feature = "turn-detect")]
use crate::core::turn_detect::{TurnDetectorConfig, assets};

/// Download and prepare all assets required for runtime execution.
#[cfg(any(
    feature = "turn-detect",
    feature = "kokoro-tts",
    feature = "whisper-stt"
))]
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

    #[cfg(feature = "whisper-stt")]
    {
        let whisper_cache = cache_path.join("whisper");
        // Check for WHISPER_MODEL environment variable, default to whisper-base
        let model_name =
            std::env::var("WHISPER_MODEL").unwrap_or_else(|_| WHISPER_DEFAULT_MODEL.to_string());

        let whisper_config = WhisperAssetConfig {
            cache_path: whisper_cache.clone(),
            model_name: model_name.clone(),
        };

        tracing::info!(
            "Preparing Whisper STT assets from HuggingFace using cache path: {:?}",
            whisper_cache
        );
        tracing::info!("Downloading Whisper model: {}", model_name);
        whisper_download_assets(&whisper_config).await?;
        tracing::info!("Whisper STT assets downloaded successfully");
    }

    Ok(())
}

#[cfg(not(any(
    feature = "turn-detect",
    feature = "kokoro-tts",
    feature = "whisper-stt"
)))]
pub async fn run() -> Result<()> {
    Err(anyhow!(
        "`sayna init` requires the `turn-detect`, `kokoro-tts`, or `whisper-stt` feature. \
         Rebuild with `--features turn-detect`, `--features kokoro-tts`, or `--features whisper-stt` to download assets."
    ))
}
