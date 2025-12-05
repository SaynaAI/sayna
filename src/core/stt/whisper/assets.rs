//! Asset management for Whisper STT
//!
//! Handles downloading and caching of Whisper model files from HuggingFace.
//! Models are cloned from HuggingFace repositories using Git LFS.
//!
//! ## Supported Models
//!
//! - `whisper-base` (~150MB) - Default model, good balance of speed/accuracy
//! - `whisper-large-v3` (~3GB) - Highest accuracy, requires more resources
//!
//! The `sayna init` command downloads the required model files.
//!
//! ## Model Files
//!
//! - `encoder_model.onnx` - Whisper encoder (audio to embeddings)
//! - `decoder_model_merged.onnx` - Whisper decoder (embeddings to tokens)
//! - `tokenizer.json` - BPE tokenizer vocabulary
//!
//! ## Reference
//!
//! Pattern follows `src/core/tts/kokoro/assets.rs` for asset management.

use anyhow::{Context, Result};
use std::path::PathBuf;
use tokio::fs;
use tracing::info;

/// HuggingFace repository URL for whisper-base
const HF_REPO_WHISPER_BASE: &str = "https://huggingface.co/onnx-community/whisper-base";

/// HuggingFace repository URL for whisper-large-v3
const HF_REPO_WHISPER_LARGE_V3: &str =
    "https://huggingface.co/onnx-community/whisper-large-v3-ONNX";

/// ONNX model subdirectory in the repository
const ONNX_DIR: &str = "onnx";

/// Encoder model filename
const ENCODER_FILENAME: &str = "encoder_model.onnx";

/// Decoder model filename (non-merged, simpler without KV-cache)
const DECODER_FILENAME: &str = "decoder_model.onnx";

/// Tokenizer filename
const TOKENIZER_FILENAME: &str = "tokenizer.json";

/// Default Whisper model
pub const DEFAULT_MODEL: &str = "whisper-base";

/// List of all supported models
pub const SUPPORTED_MODELS: &[&str] = &["whisper-base", "whisper-large-v3"];

/// Whisper asset configuration
#[derive(Debug, Clone)]
pub struct WhisperAssetConfig {
    /// Base path to the cache directory (e.g., ~/.cache/whisper)
    pub cache_path: PathBuf,
    /// Model name (whisper-base, whisper-large-v3)
    pub model_name: String,
}

impl WhisperAssetConfig {
    /// Create a new config with the specified cache path and model
    pub fn new(cache_path: PathBuf, model_name: &str) -> Self {
        Self {
            cache_path,
            model_name: model_name.to_string(),
        }
    }

    /// Get the full path to the model directory
    pub fn model_dir(&self) -> PathBuf {
        self.cache_path.join(&self.model_name)
    }
}

/// Get the HuggingFace Git URL for a model
///
/// # Arguments
/// * `model_name` - Name of the model (e.g., "whisper-base", "whisper-large-v3")
///
/// # Returns
/// * `Result<&'static str>` - The HuggingFace Git URL or error if model not found
pub fn get_model_url(model_name: &str) -> Result<&'static str> {
    match model_name {
        "whisper-base" => Ok(HF_REPO_WHISPER_BASE),
        "whisper-large-v3" => Ok(HF_REPO_WHISPER_LARGE_V3),
        _ => anyhow::bail!(
            "Unknown Whisper model '{}'. Supported models: {}",
            model_name,
            SUPPORTED_MODELS.join(", ")
        ),
    }
}

/// List all supported Whisper models
pub fn list_available_models() -> Vec<&'static str> {
    SUPPORTED_MODELS.to_vec()
}

/// Get estimated download size for a model
fn get_model_size_estimate(model_name: &str) -> &'static str {
    match model_name {
        "whisper-base" => "~150MB",
        "whisper-large-v3" => "~3GB (this may take a while)",
        _ => "unknown size",
    }
}

/// Download all Whisper assets by cloning the HuggingFace repository
///
/// # Arguments
/// * `config` - Asset configuration specifying cache path and model name
///
/// # Returns
/// * `Result<()>` - Success or error
///
/// # Notes
/// - Requires Git and Git LFS to be installed
/// - Uses shallow clone (--depth=1) to minimize download size
/// - If already cloned, pulls latest changes
pub async fn download_assets(config: &WhisperAssetConfig) -> Result<()> {
    // Validate model name
    let hf_url = get_model_url(&config.model_name)?;

    let model_dir = config.model_dir();

    // Check if already cloned
    let git_dir = model_dir.join(".git");
    if git_dir.exists() {
        info!(
            "Whisper {} repository already cloned at: {:?}",
            config.model_name, model_dir
        );
        // Pull latest changes
        info!("Pulling latest changes...");
        let output = tokio::process::Command::new("git")
            .args(["pull"])
            .current_dir(&model_dir)
            .output()
            .await
            .context("Failed to run git pull")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git pull failed: {}", stderr);
        }
        info!(
            "Whisper {} assets up to date at: {:?}",
            config.model_name, model_dir
        );
        return Ok(());
    }

    // Create parent directory (cache_path, not model_dir)
    if let Some(parent) = model_dir.parent() {
        fs::create_dir_all(parent).await?;
    }

    // Clone the repository with Git LFS
    let size_estimate = get_model_size_estimate(&config.model_name);
    info!(
        "Cloning Whisper {} repository from HuggingFace to {:?}...",
        config.model_name, model_dir
    );
    info!(
        "This may take a while (downloading {} of model files)...",
        size_estimate
    );

    let output = tokio::process::Command::new("git")
        .args(["clone", "--depth=1", hf_url, model_dir.to_str().unwrap()])
        .output()
        .await
        .context("Failed to run git clone. Make sure git and git-lfs are installed.")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "git clone failed: {}. Make sure git-lfs is installed: https://git-lfs.com",
            stderr
        );
    }

    // Verify essential files exist
    verify_assets(config).await?;

    info!(
        "Whisper {} assets downloaded to: {:?}",
        config.model_name, model_dir
    );
    Ok(())
}

/// Verify that all required asset files exist
async fn verify_assets(config: &WhisperAssetConfig) -> Result<()> {
    let model_dir = config.model_dir();

    // Check encoder
    let encoder = model_dir.join(ONNX_DIR).join(ENCODER_FILENAME);
    if !encoder.exists() {
        anyhow::bail!(
            "Encoder model not found at {:?}. The repository may be incomplete.",
            encoder
        );
    }

    // Check decoder
    let decoder = model_dir.join(ONNX_DIR).join(DECODER_FILENAME);
    if !decoder.exists() {
        anyhow::bail!(
            "Decoder model not found at {:?}. The repository may be incomplete.",
            decoder
        );
    }

    // Check tokenizer
    let tokenizer = model_dir.join(TOKENIZER_FILENAME);
    if !tokenizer.exists() {
        anyhow::bail!(
            "Tokenizer not found at {:?}. The repository may be incomplete.",
            tokenizer
        );
    }

    info!("All required Whisper assets verified successfully");
    Ok(())
}

/// Get the path to the encoder model file
///
/// # Arguments
/// * `config` - Asset configuration
///
/// # Returns
/// * `Result<PathBuf>` - Path to encoder model or error if not found
pub fn encoder_model_path(config: &WhisperAssetConfig) -> Result<PathBuf> {
    let path = config.model_dir().join(ONNX_DIR).join(ENCODER_FILENAME);
    if path.exists() {
        Ok(path)
    } else {
        anyhow::bail!(
            "Whisper encoder model not found at {:?}. Run `sayna init` to download assets.",
            path
        )
    }
}

/// Get the path to the decoder model file
///
/// # Arguments
/// * `config` - Asset configuration
///
/// # Returns
/// * `Result<PathBuf>` - Path to decoder model or error if not found
pub fn decoder_model_path(config: &WhisperAssetConfig) -> Result<PathBuf> {
    let path = config.model_dir().join(ONNX_DIR).join(DECODER_FILENAME);
    if path.exists() {
        Ok(path)
    } else {
        anyhow::bail!(
            "Whisper decoder model not found at {:?}. Run `sayna init` to download assets.",
            path
        )
    }
}

/// Get the path to the tokenizer file
///
/// # Arguments
/// * `config` - Asset configuration
///
/// # Returns
/// * `Result<PathBuf>` - Path to tokenizer file or error if not found
pub fn tokenizer_path(config: &WhisperAssetConfig) -> Result<PathBuf> {
    let path = config.model_dir().join(TOKENIZER_FILENAME);
    if path.exists() {
        Ok(path)
    } else {
        anyhow::bail!(
            "Whisper tokenizer not found at {:?}. Run `sayna init` to download assets.",
            path
        )
    }
}

/// Get the path to the config.json file
///
/// # Arguments
/// * `config` - Asset configuration
///
/// # Returns
/// * `Result<PathBuf>` - Path to config file or error if not found
pub fn config_path(config: &WhisperAssetConfig) -> Result<PathBuf> {
    let path = config.model_dir().join("config.json");
    if path.exists() {
        Ok(path)
    } else {
        anyhow::bail!(
            "Whisper config not found at {:?}. Run `sayna init` to download assets.",
            path
        )
    }
}

/// Check if all required assets are available
///
/// # Arguments
/// * `config` - Asset configuration
///
/// # Returns
/// * `bool` - True if all required files exist
pub fn are_assets_available(config: &WhisperAssetConfig) -> bool {
    encoder_model_path(config).is_ok()
        && decoder_model_path(config).is_ok()
        && tokenizer_path(config).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_new_config() {
        let dir = tempdir().unwrap();
        let config = WhisperAssetConfig::new(dir.path().to_path_buf(), "whisper-base");
        assert_eq!(config.cache_path, dir.path().to_path_buf());
        assert_eq!(config.model_name, "whisper-base");
    }

    #[test]
    fn test_new_config_large_model() {
        let dir = tempdir().unwrap();
        let config = WhisperAssetConfig::new(dir.path().to_path_buf(), "whisper-large-v3");
        assert_eq!(config.model_name, "whisper-large-v3");
    }

    #[test]
    fn test_model_dir() {
        let dir = tempdir().unwrap();
        let config = WhisperAssetConfig {
            cache_path: dir.path().to_path_buf(),
            model_name: "whisper-base".to_string(),
        };
        assert_eq!(config.model_dir(), dir.path().join("whisper-base"));
    }

    #[test]
    fn test_get_model_url_whisper_base() {
        let url = get_model_url("whisper-base").unwrap();
        assert!(url.contains("huggingface.co"));
        assert!(url.contains("whisper-base"));
    }

    #[test]
    fn test_get_model_url_whisper_large() {
        let url = get_model_url("whisper-large-v3").unwrap();
        assert!(url.contains("huggingface.co"));
        assert!(url.contains("whisper-large-v3"));
    }

    #[test]
    fn test_get_model_url_invalid() {
        let result = get_model_url("invalid-model");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Unknown Whisper model"));
        assert!(err.contains("whisper-base"));
        assert!(err.contains("whisper-large-v3"));
    }

    #[test]
    fn test_list_available_models() {
        let models = list_available_models();
        assert!(models.contains(&"whisper-base"));
        assert!(models.contains(&"whisper-large-v3"));
    }

    #[test]
    fn test_encoder_model_path_missing() {
        let dir = tempdir().unwrap();
        let config = WhisperAssetConfig {
            cache_path: dir.path().to_path_buf(),
            model_name: "whisper-base".to_string(),
        };

        let result = encoder_model_path(&config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("sayna init"));
    }

    #[test]
    fn test_encoder_model_path_exists() {
        let dir = tempdir().unwrap();
        let model_dir = dir.path().join("whisper-base").join(ONNX_DIR);
        std::fs::create_dir_all(&model_dir).unwrap();
        let model_file = model_dir.join(ENCODER_FILENAME);
        std::fs::write(&model_file, "test").unwrap();

        let config = WhisperAssetConfig {
            cache_path: dir.path().to_path_buf(),
            model_name: "whisper-base".to_string(),
        };

        let result = encoder_model_path(&config);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), model_file);
    }

    #[test]
    fn test_decoder_model_path_missing() {
        let dir = tempdir().unwrap();
        let config = WhisperAssetConfig {
            cache_path: dir.path().to_path_buf(),
            model_name: "whisper-base".to_string(),
        };

        let result = decoder_model_path(&config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("sayna init"));
    }

    #[test]
    fn test_decoder_model_path_exists() {
        let dir = tempdir().unwrap();
        let model_dir = dir.path().join("whisper-base").join(ONNX_DIR);
        std::fs::create_dir_all(&model_dir).unwrap();
        let model_file = model_dir.join(DECODER_FILENAME);
        std::fs::write(&model_file, "test").unwrap();

        let config = WhisperAssetConfig {
            cache_path: dir.path().to_path_buf(),
            model_name: "whisper-base".to_string(),
        };

        let result = decoder_model_path(&config);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), model_file);
    }

    #[test]
    fn test_tokenizer_path_missing() {
        let dir = tempdir().unwrap();
        let config = WhisperAssetConfig {
            cache_path: dir.path().to_path_buf(),
            model_name: "whisper-base".to_string(),
        };

        let result = tokenizer_path(&config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("sayna init"));
    }

    #[test]
    fn test_tokenizer_path_exists() {
        let dir = tempdir().unwrap();
        let model_dir = dir.path().join("whisper-base");
        std::fs::create_dir_all(&model_dir).unwrap();
        let tokenizer_file = model_dir.join(TOKENIZER_FILENAME);
        std::fs::write(&tokenizer_file, "{}").unwrap();

        let config = WhisperAssetConfig {
            cache_path: dir.path().to_path_buf(),
            model_name: "whisper-base".to_string(),
        };

        let result = tokenizer_path(&config);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), tokenizer_file);
    }

    #[test]
    fn test_config_path_missing() {
        let dir = tempdir().unwrap();
        let config = WhisperAssetConfig {
            cache_path: dir.path().to_path_buf(),
            model_name: "whisper-base".to_string(),
        };

        let result = config_path(&config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("sayna init"));
    }

    #[test]
    fn test_config_path_exists() {
        let dir = tempdir().unwrap();
        let model_dir = dir.path().join("whisper-base");
        std::fs::create_dir_all(&model_dir).unwrap();
        let config_file = model_dir.join("config.json");
        std::fs::write(&config_file, "{}").unwrap();

        let config = WhisperAssetConfig {
            cache_path: dir.path().to_path_buf(),
            model_name: "whisper-base".to_string(),
        };

        let result = config_path(&config);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), config_file);
    }

    #[test]
    fn test_are_assets_available_false() {
        let dir = tempdir().unwrap();
        let config = WhisperAssetConfig {
            cache_path: dir.path().to_path_buf(),
            model_name: "whisper-base".to_string(),
        };

        assert!(!are_assets_available(&config));
    }

    #[test]
    fn test_are_assets_available_partial() {
        let dir = tempdir().unwrap();
        let model_dir = dir.path().join("whisper-base");
        let onnx_dir = model_dir.join(ONNX_DIR);
        std::fs::create_dir_all(&onnx_dir).unwrap();

        // Only create encoder
        std::fs::write(onnx_dir.join(ENCODER_FILENAME), "test").unwrap();

        let config = WhisperAssetConfig {
            cache_path: dir.path().to_path_buf(),
            model_name: "whisper-base".to_string(),
        };

        // Should be false because decoder and tokenizer are missing
        assert!(!are_assets_available(&config));
    }

    #[test]
    fn test_are_assets_available_true() {
        let dir = tempdir().unwrap();
        let model_dir = dir.path().join("whisper-base");
        let onnx_dir = model_dir.join(ONNX_DIR);
        std::fs::create_dir_all(&onnx_dir).unwrap();

        // Create all required files
        std::fs::write(onnx_dir.join(ENCODER_FILENAME), "test").unwrap();
        std::fs::write(onnx_dir.join(DECODER_FILENAME), "test").unwrap();
        std::fs::write(model_dir.join(TOKENIZER_FILENAME), "{}").unwrap();

        let config = WhisperAssetConfig {
            cache_path: dir.path().to_path_buf(),
            model_name: "whisper-base".to_string(),
        };

        assert!(are_assets_available(&config));
    }

    #[test]
    fn test_supported_models_constant() {
        assert!(SUPPORTED_MODELS.contains(&"whisper-base"));
        assert!(SUPPORTED_MODELS.contains(&"whisper-large-v3"));
    }

    #[test]
    fn test_default_model_constant() {
        assert_eq!(DEFAULT_MODEL, "whisper-base");
        // Default model should be in supported models
        assert!(SUPPORTED_MODELS.contains(&DEFAULT_MODEL));
    }

    #[test]
    fn test_get_model_size_estimate() {
        assert!(get_model_size_estimate("whisper-base").contains("150MB"));
        assert!(get_model_size_estimate("whisper-large-v3").contains("3GB"));
        assert_eq!(get_model_size_estimate("unknown"), "unknown size");
    }
}
