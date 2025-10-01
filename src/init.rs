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

use anyhow::anyhow;
use anyhow::{Context, Result};
use tracing::info;

use crate::config::ServerConfig;
use crate::core::turn_detect::{TurnDetectorConfig, assets};

/// Download and prepare all assets required for runtime execution.
pub async fn run() -> Result<()> {
    let config = ServerConfig::from_env().map_err(|e| anyhow!(e.to_string()))?;
    let cache_path = config
        .cache_path
        .as_ref()
        .context("CACHE_PATH environment variable must be set to run `sayna init`")?
        .clone();

    let mut turn_config = TurnDetectorConfig::default();
    turn_config.cache_path = Some(cache_path.clone());

    info!(
        "Preparing turn detector assets using cache path: {:?}",
        cache_path
    );
    assets::download_assets(&turn_config).await?;

    info!("Turn detector assets downloaded successfully");

    Ok(())
}
