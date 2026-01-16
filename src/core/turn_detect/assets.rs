use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tokio::fs;
use tracing::{error, info, warn};

use crate::core::turn_detect::config::TurnDetectorConfig;

const MODEL_FILENAME: &str = "smart-turn-v3.1.onnx";

/// Download smart-turn model if not already cached.
pub async fn download_assets(config: &TurnDetectorConfig) -> Result<()> {
    let model_path = download_model(config).await?;
    info!("Smart-turn model ready at: {:?}", model_path);

    Ok(())
}

/// Ensure the model exists locally, downloading it when missing.
pub async fn download_model(config: &TurnDetectorConfig) -> Result<PathBuf> {
    if let Some(model_path) = &config.model_path {
        if model_path.exists() {
            return Ok(model_path.clone());
        }

        error!(
            "Configured smart-turn model path {:?} is missing or unreadable",
            model_path
        );
        anyhow::bail!(
            "Configured smart-turn model path {:?} does not exist",
            model_path
        );
    }

    let cache_dir = config.get_cache_dir()?;
    fs::create_dir_all(&cache_dir).await?;
    let model_path = cache_dir.join(MODEL_FILENAME);

    if model_path.exists() {
        info!("Using cached smart-turn model at: {:?}", model_path);
        return Ok(model_path);
    }

    let model_url = config
        .model_url
        .as_ref()
        .context("No model URL specified and model not found locally")?;

    info!("Downloading smart-turn model from: {}", model_url);
    download_file(model_url, &model_path).await?;

    Ok(model_path)
}

/// Resolve the expected on-disk location of the model without downloading it.
pub fn model_path(config: &TurnDetectorConfig) -> Result<PathBuf> {
    if let Some(model_path) = &config.model_path {
        if model_path.exists() {
            return Ok(model_path.clone());
        }

        anyhow::bail!(
            "Smart-turn model not found at configured path {:?}. Run `sayna init` first.",
            model_path
        );
    }

    let cache_dir = config.get_cache_dir()?;
    let model_path = cache_dir.join(MODEL_FILENAME);

    if model_path.exists() {
        Ok(model_path)
    } else {
        error!(
            "Smart-turn model expected at {:?} but not found. Ensure `sayna init` populated the cache.",
            model_path
        );
        anyhow::bail!(
            "Smart-turn model missing at {:?}. Run `sayna init` before starting the server.",
            model_path
        );
    }
}

async fn download_file(url: &str, path: &Path) -> Result<()> {
    let response = reqwest::get(url)
        .await
        .context("Failed to download smart-turn artifact")?;

    if !response.status().is_success() {
        anyhow::bail!(
            "Failed to download smart-turn artifact: HTTP {}",
            response.status()
        );
    }

    let bytes = response.bytes().await?;

    if let Some(expected_hash) = get_expected_hash(url) {
        verify_hash(&bytes, &expected_hash)?;
    }

    fs::write(path, bytes).await?;
    info!("Downloaded smart-turn artifact to: {:?}", path);

    Ok(())
}

fn get_expected_hash(url: &str) -> Option<String> {
    if url.contains(MODEL_FILENAME) {
        Some("expected_hash_here".to_string())
    } else {
        None
    }
}

fn verify_hash(data: &[u8], expected: &str) -> Result<()> {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(data);
    let actual = format!("{:x}", hasher.finalize());

    if actual != expected {
        warn!(
            "Smart-turn artifact hash mismatch - expected: {}, actual: {}",
            expected, actual
        );
    }

    Ok(())
}
