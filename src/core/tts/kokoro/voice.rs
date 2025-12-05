//! Voice style management for Kokoro TTS
//!
//! This module handles loading and managing voice style embeddings.
//! Voice files are individual .bin files from HuggingFace (NPY format).

use crate::core::tts::{TTSError, TTSResult};
use std::collections::HashMap;
use std::path::Path;

#[cfg(feature = "kokoro-tts")]
use ndarray_npy::read_npy;
#[cfg(feature = "kokoro-tts")]
use ndarray16::Array3;

/// Voice style embedding dimension
pub const STYLE_DIM: usize = 256;

/// Maximum token length supported by voice styles
pub const MAX_TOKENS: usize = 511;

/// Voice style entry containing embeddings for different token lengths
#[derive(Clone)]
pub struct VoiceStyle {
    /// Name of the voice
    #[allow(dead_code)]
    pub name: String,
    /// Style embeddings indexed by token count [0..511][1][256]
    embeddings: Vec<[[f32; STYLE_DIM]; 1]>,
}

impl VoiceStyle {
    /// Get the style embedding for a specific token count
    pub fn get_embedding(&self, token_count: usize) -> TTSResult<Vec<f32>> {
        if self.embeddings.is_empty() {
            return Err(TTSError::InternalError(
                "Voice style has no embeddings".to_string(),
            ));
        }
        // Clamp to both MAX_TOKENS and actual embedding length
        let max_idx = self.embeddings.len().min(MAX_TOKENS) - 1;
        let idx = token_count.min(max_idx);
        Ok(self.embeddings[idx][0].to_vec())
    }
}

/// Manager for voice styles
pub struct VoiceManager {
    styles: HashMap<String, VoiceStyle>,
}

impl VoiceManager {
    /// Load a single voice file (.bin in NPY format)
    #[cfg(feature = "kokoro-tts")]
    pub fn load_single<P: AsRef<Path>>(path: P, voice_name: &str) -> TTSResult<Self> {
        let path = path.as_ref();

        if !path.exists() {
            return Err(TTSError::InvalidConfiguration(format!(
                "Voice file not found: {}. Run 'sayna init' to download.",
                path.display()
            )));
        }

        let voice_data: Array3<f32> = read_npy(path)
            .map_err(|e| TTSError::InternalError(format!("Failed to read voice file: {}", e)))?;

        // Convert ndarray to our internal format
        let shape = voice_data.shape();
        if shape.len() != 3 || shape[1] != 1 || shape[2] != STYLE_DIM {
            return Err(TTSError::InvalidConfiguration(format!(
                "Voice file has unexpected shape {:?}, expected [N, 1, {}]",
                shape, STYLE_DIM
            )));
        }

        let mut embeddings = vec![[[0.0f32; STYLE_DIM]; 1]; shape[0]];

        for i in 0..shape[0] {
            for k in 0..STYLE_DIM {
                embeddings[i][0][k] = voice_data[[i, 0, k]];
            }
        }

        let mut styles = HashMap::new();
        styles.insert(
            voice_name.to_string(),
            VoiceStyle {
                name: voice_name.to_string(),
                embeddings,
            },
        );

        tracing::info!("Loaded voice style: {}", voice_name);

        Ok(Self { styles })
    }

    /// Load a single voice file (stub when feature is disabled)
    #[cfg(not(feature = "kokoro-tts"))]
    pub fn load_single<P: AsRef<Path>>(_path: P, _voice_name: &str) -> TTSResult<Self> {
        Err(TTSError::InvalidConfiguration(
            "Kokoro TTS feature is not enabled".to_string(),
        ))
    }

    /// Get the style embedding for a voice and token count
    pub fn get_style(&self, voice_name: &str, token_count: usize) -> TTSResult<Vec<Vec<f32>>> {
        let voice = self.styles.get(voice_name).ok_or_else(|| {
            TTSError::InvalidConfiguration(format!(
                "Voice '{}' not found. Available: {:?}",
                voice_name,
                self.get_available_voices()
            ))
        })?;

        let embedding = voice.get_embedding(token_count)?;
        Ok(vec![embedding])
    }

    /// Get list of available voice names
    pub fn get_available_voices(&self) -> Vec<String> {
        let mut voices: Vec<String> = self.styles.keys().cloned().collect();
        voices.sort();
        voices
    }

    /// Check if a voice exists
    #[allow(dead_code)]
    pub fn has_voice(&self, name: &str) -> bool {
        self.styles.contains_key(name)
    }

    /// Get the number of loaded voices
    #[allow(dead_code)]
    pub fn voice_count(&self) -> usize {
        self.styles.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a test VoiceManager with mock voice data
    fn create_test_manager() -> VoiceManager {
        let mut styles = HashMap::new();

        // Create mock voice with known embeddings
        let mut embeddings = vec![[[0.0f32; STYLE_DIM]; 1]; MAX_TOKENS];
        for (j, embed) in embeddings.iter_mut().enumerate() {
            embed[0][0] = 0.0; // voice index
            embed[0][1] = j as f32; // token index
            for (k, val) in embed[0].iter_mut().enumerate().skip(2) {
                *val = k as f32 * 0.001;
            }
        }

        styles.insert(
            "af_bella".to_string(),
            VoiceStyle {
                name: "af_bella".to_string(),
                embeddings,
            },
        );

        VoiceManager { styles }
    }

    #[test]
    fn test_style_dim() {
        assert_eq!(STYLE_DIM, 256);
    }

    #[test]
    fn test_max_tokens() {
        assert_eq!(MAX_TOKENS, 511);
    }

    #[test]
    fn test_voice_style_get_embedding() {
        let mut embeddings = vec![[[0.0f32; STYLE_DIM]; 1]; 100];
        embeddings[50][0][0] = 1.0;

        let style = VoiceStyle {
            name: "test".to_string(),
            embeddings,
        };

        let result = style.get_embedding(50);
        assert!(result.is_ok());
        let embedding = result.unwrap();
        assert_eq!(embedding.len(), STYLE_DIM);
        assert_eq!(embedding[0], 1.0);
    }

    #[test]
    fn test_voice_style_clamp() {
        let embeddings = vec![[[0.5f32; STYLE_DIM]; 1]; 100];
        let style = VoiceStyle {
            name: "test".to_string(),
            embeddings,
        };

        // Token count > len should clamp to len - 1
        let result = style.get_embedding(1000);
        assert!(result.is_ok());
    }

    #[test]
    fn test_voice_style_empty_embeddings() {
        let style = VoiceStyle {
            name: "empty".to_string(),
            embeddings: vec![],
        };

        let result = style.get_embedding(0);
        assert!(result.is_err());
    }

    #[test]
    fn test_voice_manager_get_style() {
        let manager = create_test_manager();

        let result = manager.get_style("af_bella", 100);
        assert!(result.is_ok());

        let style = result.unwrap();
        assert_eq!(style.len(), 1);
        assert_eq!(style[0].len(), STYLE_DIM);
    }

    #[test]
    fn test_voice_manager_invalid_voice() {
        let manager = create_test_manager();

        let result = manager.get_style("nonexistent", 100);
        assert!(result.is_err());
    }

    #[test]
    fn test_voice_manager_token_clamping() {
        let manager = create_test_manager();

        // Should not panic with very large token length
        let result = manager.get_style("af_bella", 10000);
        assert!(result.is_ok());

        let style = result.unwrap();
        assert_eq!(style[0].len(), STYLE_DIM);
        assert_eq!(style[0][1], (MAX_TOKENS - 1) as f32);
    }

    #[test]
    fn test_voice_manager_has_voice() {
        let manager = create_test_manager();
        assert!(manager.has_voice("af_bella"));
        assert!(!manager.has_voice("nonexistent"));
    }

    #[test]
    fn test_voice_manager_voice_count() {
        let manager = create_test_manager();
        assert_eq!(manager.voice_count(), 1);
    }

    #[test]
    fn test_voice_manager_available_voices() {
        let manager = create_test_manager();
        let voices = manager.get_available_voices();
        assert_eq!(voices.len(), 1);
        assert_eq!(voices[0], "af_bella");
    }
}
