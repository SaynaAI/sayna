//! Asset management for Kokoro TTS
//!
//! Handles cloning and caching of Kokoro model and voice files from HuggingFace.
//! All assets come from: https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX-timestamped
//!
//! The `sayna init` command clones the entire repository using Git LFS.

use anyhow::{Context, Result};
use std::path::PathBuf;
use tokio::fs;
use tracing::info;

/// HuggingFace Git repository URL
const HF_GIT_REPO: &str = "https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX-timestamped";

/// Model filename (quantized for optimal size/performance balance - 92MB)
const MODEL_FILENAME: &str = "model_quantized.onnx";

/// ONNX model subdirectory in the repository
const ONNX_DIR: &str = "onnx";

/// Voices directory name
const VOICES_DIR: &str = "voices";

/// Default voice for Kokoro TTS
pub const DEFAULT_VOICE: &str = "af_bella";

/// Kokoro asset configuration
#[derive(Debug, Clone)]
pub struct KokoroAssetConfig {
    /// Path to the cache directory (must include the "kokoro" subdirectory)
    pub cache_path: PathBuf,
}

impl KokoroAssetConfig {
    /// Create a new config with the specified cache path
    pub fn new(cache_path: PathBuf) -> Self {
        Self { cache_path }
    }
}

/// Download all Kokoro assets by cloning the HuggingFace repository
pub async fn download_assets(config: &KokoroAssetConfig) -> Result<()> {
    let cache_path = config.cache_path.clone();

    // Check if already cloned
    let git_dir = cache_path.join(".git");
    if git_dir.exists() {
        info!("Kokoro repository already cloned at: {:?}", cache_path);
        // Pull latest changes
        info!("Pulling latest changes...");
        let output = tokio::process::Command::new("git")
            .args(["pull"])
            .current_dir(&cache_path)
            .output()
            .await
            .context("Failed to run git pull")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git pull failed: {}", stderr);
        }
        info!("Kokoro assets up to date at: {:?}", cache_path);
        return Ok(());
    }

    // Create parent directory
    if let Some(parent) = cache_path.parent() {
        fs::create_dir_all(parent).await?;
    }

    // Clone the repository with Git LFS
    info!(
        "Cloning Kokoro repository from HuggingFace to {:?}...",
        cache_path
    );
    info!("This may take a while (downloading ~150MB of model and voice files)...");

    let output = tokio::process::Command::new("git")
        .args([
            "clone",
            "--depth=1",
            HF_GIT_REPO,
            cache_path.to_str().unwrap(),
        ])
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

    info!("Kokoro assets downloaded to: {:?}", cache_path);
    Ok(())
}

/// Get the path to the model file
///
/// Returns an error if the model hasn't been downloaded yet.
pub fn model_path(config: &KokoroAssetConfig) -> Result<PathBuf> {
    let path = config.cache_path.join(ONNX_DIR).join(MODEL_FILENAME);
    if path.exists() {
        Ok(path)
    } else {
        anyhow::bail!(
            "Kokoro model not found at {:?}. Run `sayna init` to download assets.",
            path
        )
    }
}

/// Get the path to a voice file
///
/// Returns an error if the voice hasn't been downloaded yet.
pub fn voice_path_sync(config: &KokoroAssetConfig, voice_name: &str) -> Result<PathBuf> {
    let path = config
        .cache_path
        .join(VOICES_DIR)
        .join(format!("{}.bin", voice_name));

    if path.exists() {
        Ok(path)
    } else {
        anyhow::bail!(
            "Voice '{}' not found at {:?}. Run `sayna init` to download assets.",
            voice_name,
            path
        )
    }
}

/// Check if a voice is available (file exists in voices directory)
pub fn is_voice_available(config: &KokoroAssetConfig, voice_name: &str) -> bool {
    config
        .cache_path
        .join(VOICES_DIR)
        .join(format!("{}.bin", voice_name))
        .exists()
}

/// Get the list of available voices from the voices directory
pub fn list_available_voices(config: &KokoroAssetConfig) -> Vec<String> {
    let voices_dir = config.cache_path.join(VOICES_DIR);
    if !voices_dir.exists() {
        return Vec::new();
    }

    let mut voices: Vec<String> = std::fs::read_dir(&voices_dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.ends_with(".bin") {
                        Some(name.trim_end_matches(".bin").to_string())
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    voices.sort();
    voices
}

/// Check if Kokoro assets are available (model and at least one voice)
///
/// This is used to determine if preloading should be attempted during startup.
pub fn are_assets_available(config: &KokoroAssetConfig) -> bool {
    // Check if model exists
    let model_exists = config
        .cache_path
        .join(ONNX_DIR)
        .join(MODEL_FILENAME)
        .exists();

    // Check if default voice exists
    let default_voice_exists = is_voice_available(config, DEFAULT_VOICE);

    model_exists && default_voice_exists
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_new_config() {
        let dir = tempdir().unwrap();
        let config = KokoroAssetConfig::new(dir.path().to_path_buf());
        assert_eq!(config.cache_path, dir.path().to_path_buf());
    }

    #[test]
    fn test_hf_git_repo() {
        assert!(HF_GIT_REPO.contains("huggingface.co"));
        assert!(HF_GIT_REPO.contains("Kokoro-82M"));
    }

    #[test]
    fn test_default_voice() {
        assert_eq!(DEFAULT_VOICE, "af_bella");
    }

    #[test]
    fn test_is_voice_available() {
        let dir = tempdir().unwrap();
        let voices_dir = dir.path().join(VOICES_DIR);
        std::fs::create_dir_all(&voices_dir).unwrap();
        std::fs::write(voices_dir.join("af_bella.bin"), "test").unwrap();

        let config = KokoroAssetConfig {
            cache_path: dir.path().to_path_buf(),
        };

        assert!(is_voice_available(&config, "af_bella"));
        assert!(!is_voice_available(&config, "fake_voice"));
    }

    #[test]
    fn test_model_path_missing() {
        let dir = tempdir().unwrap();
        let config = KokoroAssetConfig {
            cache_path: dir.path().to_path_buf(),
        };

        let result = model_path(&config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("sayna init"));
    }

    #[test]
    fn test_model_path_exists() {
        let dir = tempdir().unwrap();
        let onnx_dir = dir.path().join(ONNX_DIR);
        std::fs::create_dir_all(&onnx_dir).unwrap();
        let model_file = onnx_dir.join(MODEL_FILENAME);
        std::fs::write(&model_file, "test").unwrap();

        let config = KokoroAssetConfig {
            cache_path: dir.path().to_path_buf(),
        };

        let result = model_path(&config);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), model_file);
    }

    #[test]
    fn test_voice_path_sync_missing() {
        let dir = tempdir().unwrap();
        let config = KokoroAssetConfig {
            cache_path: dir.path().to_path_buf(),
        };

        let result = voice_path_sync(&config, "af_bella");
        assert!(result.is_err());
    }

    #[test]
    fn test_voice_path_sync_exists() {
        let dir = tempdir().unwrap();
        let voices_dir = dir.path().join(VOICES_DIR);
        std::fs::create_dir_all(&voices_dir).unwrap();
        let voice_file = voices_dir.join("af_bella.bin");
        std::fs::write(&voice_file, "test").unwrap();

        let config = KokoroAssetConfig {
            cache_path: dir.path().to_path_buf(),
        };

        let result = voice_path_sync(&config, "af_bella");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), voice_file);
    }

    #[test]
    fn test_list_available_voices_empty() {
        let dir = tempdir().unwrap();
        let config = KokoroAssetConfig {
            cache_path: dir.path().to_path_buf(),
        };

        let voices = list_available_voices(&config);
        assert!(voices.is_empty());
    }

    #[test]
    fn test_list_available_voices() {
        let dir = tempdir().unwrap();
        let voices_dir = dir.path().join(VOICES_DIR);
        std::fs::create_dir_all(&voices_dir).unwrap();
        std::fs::write(voices_dir.join("af_bella.bin"), "test").unwrap();
        std::fs::write(voices_dir.join("am_adam.bin"), "test").unwrap();

        let config = KokoroAssetConfig {
            cache_path: dir.path().to_path_buf(),
        };

        let voices = list_available_voices(&config);
        assert_eq!(voices.len(), 2);
        // Voices should be sorted
        assert_eq!(voices[0], "af_bella");
        assert_eq!(voices[1], "am_adam");
    }

    #[test]
    fn test_voice_path_sync_invalid_voice() {
        let dir = tempdir().unwrap();
        let config = KokoroAssetConfig {
            cache_path: dir.path().to_path_buf(),
        };

        // Even for valid voice names, returns error if file doesn't exist
        let result = voice_path_sync(&config, "af_bella");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("sayna init"));
    }
}
