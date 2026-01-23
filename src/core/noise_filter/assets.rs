//! Asset management for DeepFilterNet models.
//!
//! This module handles downloading and extracting DeepFilterNet model files.
//! The model comes as a tarball containing three ONNX files and a config.ini.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use tar::Archive;
use tokio::fs;
use tokio::sync::Mutex as AsyncMutex;
use tracing::{error, info};

use super::config::{
    CONFIG_FILENAME, DF_DEC_MODEL_FILENAME, ENC_MODEL_FILENAME, ERB_DEC_MODEL_FILENAME,
    NoiseFilterConfig,
};

/// Global lock for coordinating asset downloads across threads.
/// Prevents race conditions when multiple threads call `ensure_assets` concurrently.
static DOWNLOAD_LOCK: LazyLock<AsyncMutex<()>> = LazyLock::new(|| AsyncMutex::new(()));

/// Required files that must be present in the model directory.
const REQUIRED_FILES: [&str; 4] = [
    ENC_MODEL_FILENAME,
    ERB_DEC_MODEL_FILENAME,
    DF_DEC_MODEL_FILENAME,
    CONFIG_FILENAME,
];

/// Download noise filter assets if not already cached.
pub async fn download_assets(config: &NoiseFilterConfig) -> Result<()> {
    let model_dir = ensure_assets(config).await?;
    info!("DeepFilterNet models ready at: {:?}", model_dir);
    Ok(())
}

/// Ensure all required assets are available, downloading if necessary.
///
/// This function uses double-checked locking to prevent race conditions when
/// multiple threads attempt to download/extract concurrently. The lock is only
/// acquired if the models are not already present.
///
/// Returns the path to the model directory containing:
/// - enc.onnx
/// - erb_dec.onnx
/// - df_dec.onnx
/// - config.ini
pub async fn ensure_assets(config: &NoiseFilterConfig) -> Result<PathBuf> {
    // Check if explicit model path is configured
    if let Some(model_path) = &config.model_path {
        if model_path.exists() && verify_model_directory(model_path)? {
            info!("Using configured model directory: {:?}", model_path);
            return Ok(model_path.clone());
        }

        error!(
            "Configured noise filter model path {:?} is missing or incomplete",
            model_path
        );
        anyhow::bail!(
            "Configured noise filter model path {:?} does not exist or is incomplete",
            model_path
        );
    }

    // Use cache directory
    let cache_dir = config.get_cache_dir()?;

    // Quick check without lock - if already cached, return immediately
    if verify_model_directory(&cache_dir)? {
        info!("Using cached DeepFilterNet models at: {:?}", cache_dir);
        return Ok(cache_dir);
    }

    // Acquire global lock for download/extract coordination
    let _guard = DOWNLOAD_LOCK.lock().await;

    // Re-check after acquiring lock (another thread may have completed download)
    if verify_model_directory(&cache_dir)? {
        info!("Using cached DeepFilterNet models at: {:?}", cache_dir);
        return Ok(cache_dir);
    }

    // Create directory (may already exist from partial download)
    fs::create_dir_all(&cache_dir).await?;

    // Download from URL
    let model_url = config
        .model_url
        .as_ref()
        .context("No model URL specified and model not found locally")?;

    info!("Downloading DeepFilterNet models from: {}", model_url);
    download_and_extract(model_url, &cache_dir).await?;

    // Verify extraction succeeded
    if !verify_model_directory(&cache_dir)? {
        anyhow::bail!(
            "Model extraction incomplete. Missing files in {:?}",
            cache_dir
        );
    }

    Ok(cache_dir)
}

/// Resolve the expected on-disk location of the model directory without downloading it.
pub fn model_path(config: &NoiseFilterConfig) -> Result<PathBuf> {
    if let Some(model_path) = &config.model_path {
        if model_path.exists() && verify_model_directory(model_path)? {
            return Ok(model_path.clone());
        }

        anyhow::bail!(
            "Noise filter model not found at configured path {:?}. Run `sayna init` first.",
            model_path
        );
    }

    let cache_dir = config.get_cache_dir()?;

    if cache_dir.exists() && verify_model_directory(&cache_dir)? {
        Ok(cache_dir)
    } else {
        error!(
            "DeepFilterNet models expected at {:?} but not found. Ensure `sayna init` populated the cache.",
            cache_dir
        );
        anyhow::bail!(
            "DeepFilterNet models missing at {:?}. Run `sayna init` before starting the server.",
            cache_dir
        );
    }
}

/// Verify that all required model files exist in the directory.
fn verify_model_directory(dir: &Path) -> Result<bool> {
    if !dir.exists() {
        return Ok(false);
    }

    for filename in &REQUIRED_FILES {
        let file_path = dir.join(filename);
        if !file_path.exists() {
            return Ok(false);
        }
    }

    Ok(true)
}

/// Download the model tarball and extract it to the cache directory.
async fn download_and_extract(url: &str, cache_dir: &Path) -> Result<()> {
    let response = reqwest::get(url)
        .await
        .context("Failed to download DeepFilterNet model tarball")?;

    if !response.status().is_success() {
        anyhow::bail!(
            "Failed to download DeepFilterNet model: HTTP {}",
            response.status()
        );
    }

    let bytes = response.bytes().await?;
    let file_size = bytes.len();

    info!(
        "Downloaded DeepFilterNet model tarball ({} bytes), extracting...",
        file_size
    );

    // Extract the tarball
    extract_tarball(&bytes, cache_dir)?;

    info!("Extracted DeepFilterNet models to: {:?}", cache_dir);
    Ok(())
}

/// Extract a gzipped tarball to the specified directory.
///
/// The tarball structure is expected to be:
/// DeepFilterNet3_ll_onnx/
/// ├── enc.onnx
/// ├── erb_dec.onnx
/// ├── df_dec.onnx
/// └── config.ini
///
/// Files are extracted directly to the cache directory, stripping the top-level directory.
/// Uses atomic writes (temp file + rename) to prevent corruption from interrupted extractions.
fn extract_tarball(data: &[u8], dest_dir: &Path) -> Result<()> {
    let gz = GzDecoder::new(data);
    let mut archive = Archive::new(gz);

    for entry in archive.entries()? {
        let mut entry = entry?;

        // Get the filename, stripping any directory prefix
        // Clone the path to avoid borrow conflicts
        let filename = {
            let path = entry.path()?;
            path.file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
        };

        let filename = match filename {
            Some(f) => f,
            None => continue,
        };

        // Only extract required files
        if !REQUIRED_FILES.contains(&filename.as_str()) {
            continue;
        }

        // Use atomic write: temp file + rename
        let dest_path = dest_dir.join(&filename);
        let temp_path = dest_dir.join(format!(".{}.tmp", filename));

        // Read the file contents
        let mut contents = Vec::new();
        entry.read_to_end(&mut contents)?;

        // Write to temp file
        std::fs::write(&temp_path, &contents)
            .with_context(|| format!("Failed to write temp file {:?}", temp_path))?;

        // Atomic rename
        std::fs::rename(&temp_path, &dest_path)
            .with_context(|| format!("Failed to rename {:?} to {:?}", temp_path, dest_path))?;

        info!("Extracted: {}", filename);
    }

    Ok(())
}
