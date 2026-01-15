//! Model download and caching logic for Silero VAD.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use tokio::fs;
use tracing::{error, info, warn};

use super::config::SileroVADConfig;

const MODEL_FILENAME: &str = "silero_vad.onnx";

/// Expected SHA256 hash of the Silero VAD model.
const MODEL_HASH: &str = ""; // TODO: Add hash after testing

/// Download VAD model if not already cached.
pub async fn download_assets(config: &SileroVADConfig) -> Result<()> {
    let model_path = download_model(config).await?;
    info!("Silero-VAD model ready at: {:?}", model_path);
    Ok(())
}

/// Ensure the model exists locally, downloading it when missing.
pub async fn download_model(config: &SileroVADConfig) -> Result<PathBuf> {
    // Check if model_path is explicitly configured and exists
    if let Some(model_path) = &config.model_path {
        if model_path.exists() {
            return Ok(model_path.clone());
        }
        error!(
            "Configured Silero-VAD model path {:?} is missing or unreadable",
            model_path
        );
        anyhow::bail!(
            "Configured Silero-VAD model path {:?} does not exist",
            model_path
        );
    }

    // Use cache directory
    let cache_dir = config.get_cache_dir()?;
    fs::create_dir_all(&cache_dir).await?;
    let model_path = cache_dir.join(MODEL_FILENAME);

    // Check if already cached
    if model_path.exists() {
        info!("Using cached Silero-VAD model at: {:?}", model_path);
        return Ok(model_path);
    }

    // Download from URL
    let model_url = config
        .model_url
        .as_ref()
        .context("No model URL specified and model not found locally")?;

    info!("Downloading Silero-VAD model from: {}", model_url);
    download_file(model_url, &model_path).await?;

    Ok(model_path)
}

/// Resolve the expected on-disk location of the model without downloading it.
pub fn model_path(config: &SileroVADConfig) -> Result<PathBuf> {
    if let Some(model_path) = &config.model_path {
        if model_path.exists() {
            return Ok(model_path.clone());
        }
        anyhow::bail!(
            "Silero-VAD model not found at configured path {:?}. Run `sayna init` first.",
            model_path
        );
    }

    let cache_dir = config.get_cache_dir()?;
    let model_path = cache_dir.join(MODEL_FILENAME);

    if model_path.exists() {
        Ok(model_path)
    } else {
        error!(
            "Silero-VAD model expected at {:?} but not found. Ensure `sayna init` populated the cache.",
            model_path
        );
        anyhow::bail!(
            "Silero-VAD model missing at {:?}. Run `sayna init` before starting the server.",
            model_path
        );
    }
}

async fn download_file(url: &str, path: &Path) -> Result<()> {
    let response = reqwest::get(url)
        .await
        .context("Failed to download Silero-VAD model")?;

    if !response.status().is_success() {
        anyhow::bail!(
            "Failed to download Silero-VAD model: HTTP {}",
            response.status()
        );
    }

    let bytes = response.bytes().await?;
    let file_size = bytes.len();

    // Basic validation - Silero-VAD model should be around 1.5-2MB
    if file_size < 1_000_000 {
        warn!(
            "Silero-VAD model file is suspiciously small: {} bytes",
            file_size
        );
    }

    // Verify hash if available
    if !MODEL_HASH.is_empty() {
        verify_hash(&bytes, MODEL_HASH)?;
    }

    fs::write(path, bytes).await?;
    info!(
        "Downloaded Silero-VAD model to: {:?} ({} bytes)",
        path, file_size
    );

    Ok(())
}

/// Verify the SHA256 hash of downloaded data.
fn verify_hash(data: &[u8], expected: &str) -> Result<()> {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let actual = format!("{:x}", hasher.finalize());

    if actual != expected {
        warn!(
            "Silero-VAD model hash mismatch - expected: {}, actual: {}",
            expected, actual
        );
    }

    Ok(())
}
