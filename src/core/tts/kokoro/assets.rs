//! Asset management for Kokoro TTS
//!
//! Handles downloading and caching of Kokoro model and voice files.
//! This module follows the same pattern as `src/core/turn_detect/assets.rs`.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{error, info, warn};

/// Default model filename
pub const MODEL_FILENAME: &str = "kokoro-v1.0.onnx";

/// Timestamped model filename (supports word alignment)
pub const MODEL_TIMESTAMPED_FILENAME: &str = "kokoro-v1.0-timestamped.onnx";

/// Voice styles filename
pub const VOICES_FILENAME: &str = "voices-v1.0.bin";

/// Default model URL
pub const DEFAULT_MODEL_URL: &str = "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/kokoro-v1.0.onnx";

/// Timestamped model URL (for word alignment support)
pub const DEFAULT_MODEL_TIMESTAMPED_URL: &str = "https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX-timestamped/resolve/main/model_quantized.onnx";

/// Default voices URL
pub const DEFAULT_VOICES_URL: &str = "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/voices-v1.0.bin";

/// Kokoro asset configuration
#[derive(Debug, Clone)]
pub struct KokoroAssetConfig {
    /// Path to the cache directory (assets stored directly here)
    pub cache_path: PathBuf,
    /// URL to download the model from
    pub model_url: String,
    /// URL to download voices from
    pub voices_url: String,
    /// Whether to download the timestamped model variant
    pub use_timestamped: bool,
    /// Explicit path to model file (overrides cache path)
    pub model_path: Option<PathBuf>,
    /// Explicit path to voices file (overrides cache path)
    pub voices_path: Option<PathBuf>,
}

impl Default for KokoroAssetConfig {
    fn default() -> Self {
        Self {
            cache_path: get_default_cache_path(),
            model_url: DEFAULT_MODEL_URL.to_string(),
            voices_url: DEFAULT_VOICES_URL.to_string(),
            use_timestamped: false,
            model_path: None,
            voices_path: None,
        }
    }
}

/// Get the default cache path for Kokoro assets
pub fn get_default_cache_path() -> PathBuf {
    std::env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
        .join("kokoro")
}

/// Download all Kokoro assets (model + voices)
pub async fn download_assets(config: &KokoroAssetConfig) -> Result<()> {
    let model_path = download_model(config).await?;
    info!("Kokoro model ready at: {:?}", model_path);

    let voices_path = download_voices(config).await?;
    info!("Kokoro voices ready at: {:?}", voices_path);

    Ok(())
}

/// Download the Kokoro ONNX model if not already cached.
///
/// If an explicit `model_path` is configured and exists, returns it directly.
/// Otherwise, downloads to the cache directory.
pub async fn download_model(config: &KokoroAssetConfig) -> Result<PathBuf> {
    // Check explicit config path first
    if let Some(model_path) = &config.model_path {
        if model_path.exists() {
            return Ok(model_path.clone());
        }

        error!(
            "Configured Kokoro model path {:?} is missing or unreadable",
            model_path
        );
        anyhow::bail!(
            "Configured Kokoro model path {:?} does not exist",
            model_path
        );
    }

    // Create cache directory
    fs::create_dir_all(&config.cache_path).await?;

    let (filename, url) = if config.use_timestamped {
        (MODEL_TIMESTAMPED_FILENAME, DEFAULT_MODEL_TIMESTAMPED_URL)
    } else {
        (MODEL_FILENAME, config.model_url.as_str())
    };

    let model_path = config.cache_path.join(filename);

    if model_path.exists() {
        info!("Using cached Kokoro model at: {:?}", model_path);
        return Ok(model_path);
    }

    info!("Downloading Kokoro model from: {}", url);
    download_file(url, &model_path).await?;

    Ok(model_path)
}

/// Download the Kokoro voice styles if not already cached.
///
/// If an explicit `voices_path` is configured and exists, returns it directly.
/// Otherwise, downloads to the cache directory.
pub async fn download_voices(config: &KokoroAssetConfig) -> Result<PathBuf> {
    // Check explicit config path first
    if let Some(voices_path) = &config.voices_path {
        if voices_path.exists() {
            return Ok(voices_path.clone());
        }

        anyhow::bail!(
            "Configured Kokoro voices path {:?} does not exist",
            voices_path
        );
    }

    // Create cache directory
    fs::create_dir_all(&config.cache_path).await?;

    let voices_path = config.cache_path.join(VOICES_FILENAME);

    if voices_path.exists() {
        info!("Using cached Kokoro voices at: {:?}", voices_path);
        return Ok(voices_path);
    }

    info!("Downloading Kokoro voices from: {}", config.voices_url);
    download_file(&config.voices_url, &voices_path).await?;

    Ok(voices_path)
}

/// Resolve the expected on-disk location of the model without downloading it.
///
/// Checks explicit config path first, then falls back to cache path.
/// Returns an error if the model file doesn't exist.
pub fn model_path(config: &KokoroAssetConfig) -> Result<PathBuf> {
    // Check explicit config path first
    if let Some(model_path) = &config.model_path {
        if model_path.exists() {
            return Ok(model_path.clone());
        }

        anyhow::bail!(
            "Kokoro model not found at configured path {:?}. Run `sayna init` first.",
            model_path
        );
    }

    // Check cache path
    let filename = if config.use_timestamped {
        MODEL_TIMESTAMPED_FILENAME
    } else {
        MODEL_FILENAME
    };

    let model_path = config.cache_path.join(filename);

    if model_path.exists() {
        Ok(model_path)
    } else {
        error!(
            "Kokoro model expected at {:?} but not found. Ensure `sayna init` populated the cache.",
            model_path
        );
        anyhow::bail!(
            "Kokoro model missing at {:?}. Run `sayna init` before starting the server.",
            model_path
        );
    }
}

/// Resolve the expected on-disk location of the voices without downloading it.
///
/// Checks explicit config path first, then falls back to cache path.
/// Returns an error if the voices file doesn't exist.
pub fn voices_path(config: &KokoroAssetConfig) -> Result<PathBuf> {
    // Check explicit config path first
    if let Some(voices_path) = &config.voices_path {
        if voices_path.exists() {
            return Ok(voices_path.clone());
        }

        error!(
            "Configured Kokoro voices path {:?} is missing or unreadable",
            voices_path
        );
        anyhow::bail!(
            "Kokoro voices not found at configured path {:?}. Run `sayna init` first.",
            voices_path
        );
    }

    // Check cache path
    let voices_path = config.cache_path.join(VOICES_FILENAME);

    if voices_path.exists() {
        Ok(voices_path)
    } else {
        error!(
            "Kokoro voices expected at {:?} but not found. Ensure `sayna init` populated the cache.",
            voices_path
        );
        anyhow::bail!(
            "Kokoro voices missing at {:?}. Run `sayna init` before starting the server.",
            voices_path
        );
    }
}

/// Download a file from URL to path
async fn download_file(url: &str, path: &Path) -> Result<()> {
    let response = reqwest::get(url)
        .await
        .context("Failed to download Kokoro asset")?;

    if !response.status().is_success() {
        anyhow::bail!(
            "Failed to download Kokoro asset: HTTP {}",
            response.status()
        );
    }

    let content_length = response.content_length();
    if let Some(len) = content_length {
        info!("Downloading {} bytes...", len);
    }

    let bytes = response.bytes().await?;

    // Optional hash verification
    if let Some(expected_hash) = get_expected_hash(url) {
        verify_hash(&bytes, &expected_hash)?;
    }

    fs::write(path, bytes).await?;
    info!("Downloaded Kokoro asset to: {:?}", path);

    Ok(())
}

/// Get the expected SHA256 hash for a known file URL
///
/// Returns `None` for unknown files to skip verification.
fn get_expected_hash(url: &str) -> Option<String> {
    // Currently no known hashes - files may be updated
    // Add hashes here when we have verified checksums for releases
    if url.contains(MODEL_FILENAME) {
        // No known hash yet
        None
    } else if url.contains(VOICES_FILENAME) {
        // No known hash yet
        None
    } else {
        None
    }
}

/// Verify the SHA256 hash of downloaded data
///
/// Logs a warning if the hash doesn't match but doesn't fail.
/// This allows for file updates while still providing a verification mechanism.
fn verify_hash(data: &[u8], expected: &str) -> Result<()> {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(data);
    let actual = format!("{:x}", hasher.finalize());

    if actual != expected {
        warn!(
            "Kokoro asset hash mismatch - expected: {}, actual: {}",
            expected, actual
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_default_config() {
        let config = KokoroAssetConfig::default();
        assert!(!config.use_timestamped);
        assert!(config.model_url.contains("kokoro"));
        assert!(config.voices_url.contains("voices"));
        assert!(config.model_path.is_none());
        assert!(config.voices_path.is_none());
    }

    #[test]
    fn test_default_cache_path() {
        let path = get_default_cache_path();
        assert!(path.to_string_lossy().contains("kokoro"));
    }

    #[test]
    fn test_urls() {
        assert!(DEFAULT_MODEL_URL.contains("onnx"));
        assert!(DEFAULT_VOICES_URL.contains("voices"));
    }

    #[test]
    fn test_model_path_with_explicit_config_exists() {
        let dir = tempdir().unwrap();
        let model_file = dir.path().join("test_model.onnx");
        std::fs::write(&model_file, "test").unwrap();

        let config = KokoroAssetConfig {
            model_path: Some(model_file.clone()),
            ..Default::default()
        };

        let result = model_path(&config);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), model_file);
    }

    #[test]
    fn test_model_path_with_explicit_config_missing() {
        let config = KokoroAssetConfig {
            model_path: Some(PathBuf::from("/nonexistent/model.onnx")),
            ..Default::default()
        };

        let result = model_path(&config);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("sayna init"));
    }

    #[test]
    fn test_model_path_from_cache() {
        let dir = tempdir().unwrap();
        let model_file = dir.path().join(MODEL_FILENAME);
        std::fs::write(&model_file, "test").unwrap();

        let config = KokoroAssetConfig {
            cache_path: dir.path().to_path_buf(),
            ..Default::default()
        };

        let result = model_path(&config);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), model_file);
    }

    #[test]
    fn test_model_path_missing() {
        let dir = tempdir().unwrap();
        let config = KokoroAssetConfig {
            cache_path: dir.path().to_path_buf(),
            ..Default::default()
        };

        let result = model_path(&config);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("sayna init"));
    }

    #[test]
    fn test_voices_path_with_explicit_config_exists() {
        let dir = tempdir().unwrap();
        let voices_file = dir.path().join("test_voices.bin");
        std::fs::write(&voices_file, "test").unwrap();

        let config = KokoroAssetConfig {
            voices_path: Some(voices_file.clone()),
            ..Default::default()
        };

        let result = voices_path(&config);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), voices_file);
    }

    #[test]
    fn test_voices_path_with_explicit_config_missing() {
        let config = KokoroAssetConfig {
            voices_path: Some(PathBuf::from("/nonexistent/voices.bin")),
            ..Default::default()
        };

        let result = voices_path(&config);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("sayna init"));
    }

    #[test]
    fn test_voices_path_from_cache() {
        let dir = tempdir().unwrap();
        let voices_file = dir.path().join(VOICES_FILENAME);
        std::fs::write(&voices_file, "test").unwrap();

        let config = KokoroAssetConfig {
            cache_path: dir.path().to_path_buf(),
            ..Default::default()
        };

        let result = voices_path(&config);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), voices_file);
    }

    #[test]
    fn test_verify_hash_mismatch() {
        // Should not fail on mismatch, just warn
        let data = b"test data";
        let result = verify_hash(data, "wrong_hash");
        assert!(result.is_ok());
    }

    #[test]
    fn test_get_expected_hash_returns_none() {
        // Currently all hashes are None
        assert!(get_expected_hash("https://example.com/model.onnx").is_none());
        assert!(get_expected_hash("https://example.com/voices.bin").is_none());
    }

    #[tokio::test]
    async fn test_download_model_with_explicit_path_exists() {
        let dir = tempdir().unwrap();
        let model_file = dir.path().join("test_model.onnx");
        std::fs::write(&model_file, "test").unwrap();

        let config = KokoroAssetConfig {
            model_path: Some(model_file.clone()),
            cache_path: dir.path().to_path_buf(),
            ..Default::default()
        };

        let result = download_model(&config).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), model_file);
    }

    #[tokio::test]
    async fn test_download_model_with_explicit_path_missing() {
        let config = KokoroAssetConfig {
            model_path: Some(PathBuf::from("/nonexistent/model.onnx")),
            ..Default::default()
        };

        let result = download_model(&config).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_download_voices_with_explicit_path_exists() {
        let dir = tempdir().unwrap();
        let voices_file = dir.path().join("test_voices.bin");
        std::fs::write(&voices_file, "test").unwrap();

        let config = KokoroAssetConfig {
            voices_path: Some(voices_file.clone()),
            cache_path: dir.path().to_path_buf(),
            ..Default::default()
        };

        let result = download_voices(&config).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), voices_file);
    }

    #[tokio::test]
    async fn test_download_voices_with_explicit_path_missing() {
        let config = KokoroAssetConfig {
            voices_path: Some(PathBuf::from("/nonexistent/voices.bin")),
            ..Default::default()
        };

        let result = download_voices(&config).await;
        assert!(result.is_err());
    }
}
