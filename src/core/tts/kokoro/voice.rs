//! Voice style management for Kokoro TTS
//!
//! This module handles loading and managing voice style embeddings
//! from the voices.bin NPZ file.

use crate::core::tts::{TTSError, TTSResult};
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

#[cfg(feature = "kokoro-tts")]
use ndarray_npy::NpzReader;
#[cfg(feature = "kokoro-tts")]
use ndarray16::Array3;

/// Voice style embedding dimension
pub const STYLE_DIM: usize = 256;

/// Maximum token length supported by voice styles
pub const MAX_TOKENS: usize = 511;

/// Voice style entry containing embeddings for different token lengths
#[derive(Clone)]
pub struct VoiceStyle {
    /// Name of the voice (kept for debugging/logging purposes)
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
    /// Load voice styles from an NPZ file
    #[cfg(feature = "kokoro-tts")]
    pub fn load<P: AsRef<Path>>(path: P) -> TTSResult<Self> {
        let path = path.as_ref();

        if !path.exists() {
            return Err(TTSError::InvalidConfiguration(format!(
                "Voices file not found: {}. Run 'sayna init' to download.",
                path.display()
            )));
        }

        let file = File::open(path)
            .map_err(|e| TTSError::InternalError(format!("Failed to open voices file: {}", e)))?;

        let mut npz = NpzReader::new(file)
            .map_err(|e| TTSError::InternalError(format!("Failed to read NPZ file: {}", e)))?;

        let names = npz
            .names()
            .map_err(|e| TTSError::InternalError(format!("Failed to read NPZ names: {}", e)))?;

        let mut styles = HashMap::new();

        for name in names {
            let voice_data: Array3<f32> = npz.by_name(&name).map_err(|e| {
                TTSError::InternalError(format!("Failed to read voice '{}': {}", name, e))
            })?;

            // Convert ndarray to our internal format
            let shape = voice_data.shape();
            if shape.len() != 3 || shape[1] != 1 || shape[2] != STYLE_DIM {
                tracing::warn!("Skipping voice '{}': unexpected shape {:?}", name, shape);
                continue;
            }

            let mut embeddings = vec![[[0.0f32; STYLE_DIM]; 1]; shape[0]];

            for i in 0..shape[0] {
                for k in 0..STYLE_DIM {
                    embeddings[i][0][k] = voice_data[[i, 0, k]];
                }
            }

            styles.insert(name.clone(), VoiceStyle { name, embeddings });
        }

        if styles.is_empty() {
            return Err(TTSError::InvalidConfiguration(
                "No valid voices found in NPZ file".to_string(),
            ));
        }

        tracing::info!("Loaded {} voice styles", styles.len());
        Self::log_available_voices(&styles);

        Ok(Self { styles })
    }

    /// Load voice styles (stub when feature is disabled)
    #[cfg(not(feature = "kokoro-tts"))]
    pub fn load<P: AsRef<Path>>(_path: P) -> TTSResult<Self> {
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

    /// Get a blended style from multiple voices
    ///
    /// Format: "voice1.weight1+voice2.weight2" where weights are 0-10
    /// Example: "af_bella.5+am_adam.5" blends 50% each
    #[allow(dead_code)]
    pub fn get_mixed_style(
        &self,
        style_spec: &str,
        token_count: usize,
    ) -> TTSResult<Vec<Vec<f32>>> {
        if !style_spec.contains('+') {
            return self.get_style(style_spec, token_count);
        }

        let parts: Vec<&str> = style_spec.split('+').collect();
        let mut blended = vec![0.0f32; STYLE_DIM];
        let mut total_weight = 0.0f32;

        for part in parts {
            if let Some((name, weight_str)) = part.split_once('.') {
                let weight: f32 = weight_str.parse().unwrap_or(5.0) * 0.1;
                let voice = self.styles.get(name).ok_or_else(|| {
                    TTSError::InvalidConfiguration(format!("Voice '{}' not found", name))
                })?;

                let embedding = voice.get_embedding(token_count)?;
                for (i, &val) in embedding.iter().enumerate() {
                    blended[i] += val * weight;
                }
                total_weight += weight;
            }
        }

        // Normalize if weights don't sum to 1
        if total_weight > 0.0 && (total_weight - 1.0).abs() > 0.01 {
            for val in &mut blended {
                *val /= total_weight;
            }
        }

        Ok(vec![blended])
    }

    /// Get list of available voice names
    pub fn get_available_voices(&self) -> Vec<String> {
        let mut voices: Vec<String> = self.styles.keys().cloned().collect();
        voices.sort();
        voices
    }

    /// Log available voices grouped by category
    fn log_available_voices(styles: &HashMap<String, VoiceStyle>) {
        let mut voices: Vec<&String> = styles.keys().collect();
        voices.sort();

        // Group by prefix
        let mut grouped: HashMap<&str, Vec<&String>> = HashMap::new();

        for voice in &voices {
            let prefix = voice.get(0..2).unwrap_or("??");
            grouped.entry(prefix).or_default().push(voice);
        }

        tracing::info!("==========================================");
        tracing::info!("Available Kokoro voice styles ({} total):", voices.len());
        tracing::info!("==========================================");

        let categories = [
            ("af", "American Female"),
            ("am", "American Male"),
            ("bf", "British Female"),
            ("bm", "British Male"),
        ];

        for (prefix, category) in categories {
            if let Some(voices) = grouped.get(prefix) {
                let voice_list = voices
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                tracing::info!("{}: {}", category, voice_list);
            }
        }

        tracing::info!("==========================================");
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

/// Available Kokoro voices
#[allow(dead_code)]
pub const KOKORO_VOICES: &[(&str, &str)] = &[
    // American Female
    ("af_bella", "American Female - Bella"),
    ("af_nicole", "American Female - Nicole"),
    ("af_sarah", "American Female - Sarah"),
    ("af_sky", "American Female - Sky"),
    // American Male
    ("am_adam", "American Male - Adam"),
    ("am_michael", "American Male - Michael"),
    // British Female
    ("bf_emma", "British Female - Emma"),
    ("bf_isabella", "British Female - Isabella"),
    // British Male
    ("bm_george", "British Male - George"),
    ("bm_lewis", "British Male - Lewis"),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a test VoiceManager with mock voice data
    fn create_test_manager() -> VoiceManager {
        let mut styles = HashMap::new();

        // Create mock voices with known embeddings
        let voices = ["af_bella", "am_adam", "bf_emma", "bm_george"];

        for (i, voice_name) in voices.iter().enumerate() {
            // Create embeddings with distinct values per voice
            let mut embeddings = vec![[[0.0f32; STYLE_DIM]; 1]; MAX_TOKENS];
            for (j, embed) in embeddings.iter_mut().enumerate() {
                // Set a pattern: first element = voice index, second = token index
                embed[0][0] = i as f32;
                embed[0][1] = j as f32;
                // Fill rest with voice-specific values
                for k in 2..STYLE_DIM {
                    embed[0][k] = (i * 100 + k) as f32 * 0.001;
                }
            }

            styles.insert(
                voice_name.to_string(),
                VoiceStyle {
                    name: voice_name.to_string(),
                    embeddings,
                },
            );
        }

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
        // Set a known value
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
    fn test_kokoro_voices_list() {
        assert!(!KOKORO_VOICES.is_empty());

        // Check for expected voices
        let voice_ids: Vec<&str> = KOKORO_VOICES.iter().map(|(id, _)| *id).collect();
        assert!(voice_ids.contains(&"af_bella"));
        assert!(voice_ids.contains(&"am_adam"));
    }

    // VoiceManager tests

    #[test]
    fn test_voice_manager_voice_count() {
        let manager = create_test_manager();
        assert_eq!(manager.voice_count(), 4);
    }

    #[test]
    fn test_voice_manager_enumeration() {
        let manager = create_test_manager();
        let voices = manager.get_available_voices();

        assert_eq!(voices.len(), 4);
        // Voices should be sorted
        assert_eq!(voices[0], "af_bella");
        assert_eq!(voices[1], "am_adam");
        assert_eq!(voices[2], "bf_emma");
        assert_eq!(voices[3], "bm_george");
    }

    #[test]
    fn test_voice_manager_has_voice() {
        let manager = create_test_manager();

        assert!(manager.has_voice("af_bella"));
        assert!(manager.has_voice("am_adam"));
        assert!(!manager.has_voice("nonexistent_voice"));
        assert!(!manager.has_voice(""));
    }

    #[test]
    fn test_voice_manager_get_style() {
        let manager = create_test_manager();

        let result = manager.get_style("af_bella", 100);
        assert!(result.is_ok());

        let style = result.unwrap();
        assert_eq!(style.len(), 1); // Outer vec has 1 element
        assert_eq!(style[0].len(), STYLE_DIM); // Inner vec has 256 elements
    }

    #[test]
    fn test_voice_manager_get_style_correct_values() {
        let manager = create_test_manager();

        // af_bella is index 0, token count 100
        let result = manager.get_style("af_bella", 100).unwrap();
        assert_eq!(result[0][0], 0.0); // voice index
        assert_eq!(result[0][1], 100.0); // token index

        // am_adam is index 1, token count 50
        let result = manager.get_style("am_adam", 50).unwrap();
        assert_eq!(result[0][0], 1.0); // voice index
        assert_eq!(result[0][1], 50.0); // token index
    }

    #[test]
    fn test_voice_manager_invalid_voice() {
        let manager = create_test_manager();

        let result = manager.get_style("nonexistent_voice", 100);
        assert!(result.is_err());

        // Error message should mention the voice name
        let err = result.unwrap_err();
        assert!(err.to_string().contains("nonexistent_voice"));
    }

    #[test]
    fn test_voice_manager_token_length_clamping() {
        let manager = create_test_manager();

        // Should not panic with very large token length
        let result = manager.get_style("af_bella", 10000);
        assert!(result.is_ok());

        let style = result.unwrap();
        assert_eq!(style[0].len(), STYLE_DIM);

        // Token index should be clamped to MAX_TOKENS - 1 = 510
        assert_eq!(style[0][1], (MAX_TOKENS - 1) as f32);
    }

    #[test]
    fn test_voice_manager_token_zero() {
        let manager = create_test_manager();

        let result = manager.get_style("af_bella", 0);
        assert!(result.is_ok());

        let style = result.unwrap();
        assert_eq!(style[0][1], 0.0); // token index 0
    }

    #[test]
    fn test_voice_manager_mixed_style_single_voice() {
        let manager = create_test_manager();

        // Single voice should return same as get_style
        let single = manager.get_style("af_bella", 100).unwrap();
        let mixed = manager.get_mixed_style("af_bella", 100).unwrap();

        assert_eq!(single, mixed);
    }

    #[test]
    fn test_voice_manager_mixed_style_two_voices() {
        let manager = create_test_manager();

        // Mix af_bella (index 0) and am_adam (index 1) with 50/50
        let mixed = manager
            .get_mixed_style("af_bella.5+am_adam.5", 100)
            .unwrap();

        assert_eq!(mixed.len(), 1);
        assert_eq!(mixed[0].len(), STYLE_DIM);

        // The blended result should be different from individual voices
        let bella = manager.get_style("af_bella", 100).unwrap();
        let adam = manager.get_style("am_adam", 100).unwrap();

        // Mixed should not equal either individual voice
        assert_ne!(mixed[0], bella[0]);
        assert_ne!(mixed[0], adam[0]);
    }

    #[test]
    fn test_voice_manager_mixed_style_weighted() {
        let manager = create_test_manager();

        // 70% bella, 30% adam
        let result = manager.get_mixed_style("af_bella.7+am_adam.3", 100);
        assert!(result.is_ok());

        let mixed = result.unwrap();
        assert_eq!(mixed[0].len(), STYLE_DIM);
    }

    #[test]
    fn test_voice_manager_mixed_style_invalid_voice() {
        let manager = create_test_manager();

        let result = manager.get_mixed_style("af_bella.5+nonexistent.5", 100);
        assert!(result.is_err());
    }

    #[test]
    fn test_voice_manager_mixed_style_three_voices() {
        let manager = create_test_manager();

        // Mix three voices
        let result = manager.get_mixed_style("af_bella.3+am_adam.3+bf_emma.4", 100);
        assert!(result.is_ok());

        let mixed = result.unwrap();
        assert_eq!(mixed[0].len(), STYLE_DIM);
    }
}
