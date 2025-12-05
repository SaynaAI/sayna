//! Preloaded Kokoro TTS resources for efficient startup
//!
//! This module provides a cache for preloaded Kokoro TTS resources (model, voices, phonemizer)
//! to avoid loading them on every TTS request. Resources are loaded once during server startup
//! and shared across all TTS sessions.
//!
//! # Usage
//!
//! The `KokoroModelCache` is initialized during application startup via `CoreState` and
//! can be passed to `KokoroTTS` instances to skip resource loading.

use std::path::PathBuf;
use std::sync::Arc;

use super::model::KokoroModel;
use super::phonemizer::Phonemizer;
use super::voice::VoiceManager;
use super::{DEFAULT_VOICE, KOKORO_SAMPLE_RATE, KokoroAssetConfig, assets};
use crate::core::tts::{TTSError, TTSResult};

/// Configuration for Kokoro model cache
#[derive(Debug, Clone)]
pub struct KokoroModelCacheConfig {
    /// Cache path for Kokoro assets
    pub cache_path: PathBuf,
    /// Default voice to preload (default: "af_bella")
    pub default_voice: String,
}

impl Default for KokoroModelCacheConfig {
    fn default() -> Self {
        Self {
            cache_path: PathBuf::from("/app/cache/kokoro"),
            default_voice: DEFAULT_VOICE.to_string(),
        }
    }
}

/// Cached Kokoro TTS resources for sharing across sessions
///
/// This struct holds preloaded resources that are expensive to load:
/// - ONNX model (~87MB)
/// - Default voice embeddings (~522KB)
/// - Phonemizer (eSpeak-NG initialization)
///
/// Resources are wrapped in `Arc` for efficient sharing across TTS instances.
pub struct KokoroModelCache {
    /// Preloaded ONNX model
    pub model: Arc<KokoroModel>,
    /// Preloaded default voice
    pub default_voice: Arc<VoiceManager>,
    /// Voice name for the default voice
    pub default_voice_name: String,
    /// Preloaded phonemizer
    pub phonemizer: Arc<Phonemizer>,
    /// Asset configuration
    pub asset_config: KokoroAssetConfig,
}

impl KokoroModelCache {
    /// Initialize the Kokoro model cache
    ///
    /// This loads:
    /// 1. The ONNX model from the cache directory
    /// 2. The default voice embeddings
    /// 3. The eSpeak-NG phonemizer
    ///
    /// # Arguments
    /// * `config` - Configuration specifying cache path and default voice
    ///
    /// # Returns
    /// A `KokoroModelCache` with preloaded resources
    ///
    /// # Errors
    /// Returns an error if any resource fails to load
    #[cfg(feature = "kokoro-tts")]
    pub async fn new(config: KokoroModelCacheConfig) -> TTSResult<Self> {
        let asset_config = KokoroAssetConfig {
            cache_path: config.cache_path,
        };

        // Load model
        let model_path = assets::model_path(&asset_config)
            .map_err(|e| TTSError::InvalidConfiguration(e.to_string()))?;

        tracing::info!("Preloading Kokoro ONNX model from: {:?}", model_path);
        let model = KokoroModel::load(&model_path).await?;

        // Load default voice
        let voice_path = assets::voice_path_sync(&asset_config, &config.default_voice)
            .map_err(|e| TTSError::InvalidConfiguration(e.to_string()))?;

        tracing::info!(
            "Preloading Kokoro voice '{}' from: {:?}",
            config.default_voice,
            voice_path
        );
        let default_voice = VoiceManager::load_single(&voice_path, &config.default_voice)?;

        // Initialize phonemizer
        tracing::info!("Initializing Kokoro phonemizer (eSpeak-NG)");
        let phonemizer = Phonemizer::new()?;

        Ok(Self {
            model: Arc::new(model),
            default_voice: Arc::new(default_voice),
            default_voice_name: config.default_voice,
            phonemizer: Arc::new(phonemizer),
            asset_config,
        })
    }

    /// Stub for new() when feature is disabled
    #[cfg(not(feature = "kokoro-tts"))]
    pub async fn new(_config: KokoroModelCacheConfig) -> TTSResult<Self> {
        Err(TTSError::InvalidConfiguration(
            "Kokoro TTS feature is not enabled".to_string(),
        ))
    }

    /// Warmup the model with a full end-to-end TTS pass
    ///
    /// This ensures the model and all processing pipelines are fully loaded
    /// and ready for fast inference. We use a representative sentence to
    /// warm up all code paths including:
    /// - Phonemizer (eSpeak-NG)
    /// - Tokenizer
    /// - Voice style embedding
    /// - ONNX model inference
    /// - Audio resampling (24kHz -> 48kHz)
    /// - PCM16 conversion
    #[cfg(feature = "kokoro-tts")]
    pub async fn warmup(&self) -> TTSResult<()> {
        use super::tokenizer;

        tracing::debug!("Starting Kokoro TTS warmup with full pipeline");

        // Get a mutable reference through Arc - we need to clone for inference
        // Note: KokoroModel has internal Mutex, so this is safe
        let model = Arc::clone(&self.model);

        // Use a representative sentence to warm up more phonemes and tokens
        // This exercises more of the model's capabilities than a short "Hello"
        let warmup_text = "Welcome to the voice assistant. How can I help you today?";

        let start = std::time::Instant::now();

        // Step 1: Phonemize (warms up eSpeak-NG)
        let phonemes = self.phonemizer.phonemize(warmup_text, "a");
        if phonemes.is_empty() {
            tracing::warn!("Warmup phonemization returned empty result");
            return Ok(());
        }
        tracing::debug!(
            "Warmup phonemization: {} chars -> {} phonemes",
            warmup_text.len(),
            phonemes.len()
        );

        // Step 2: Tokenize
        let tokens = tokenizer::tokenize(&phonemes);
        if tokens.is_empty() {
            tracing::warn!("Warmup tokenization returned empty result");
            return Ok(());
        }
        tracing::debug!("Warmup tokenization: {} tokens", tokens.len());

        // Step 3: Get style embedding
        let style = self
            .default_voice
            .get_style(&self.default_voice_name, tokens.len())?;

        // Step 4: Run inference (main ONNX warmup)
        // We need to use unsafe to get mutable reference since model.infer() requires &mut self
        // But KokoroModel internally uses Mutex, so concurrent access is safe
        let model_ptr = Arc::as_ptr(&model) as *mut KokoroModel;
        let model_mut = unsafe { &mut *model_ptr };

        let inference_start = std::time::Instant::now();
        let audio = model_mut.infer(tokens, style, 1.0).await?;
        let inference_elapsed = inference_start.elapsed();
        tracing::debug!(
            "Warmup inference: {} samples in {:?}",
            audio.len(),
            inference_elapsed
        );

        // Step 5: Resample to 48kHz (common LiveKit target rate)
        // This warms up the resampling code path and memory allocations
        let resample_start = std::time::Instant::now();
        let resampled = Self::resample_warmup(&audio, KOKORO_SAMPLE_RATE, 48000);
        let resample_elapsed = resample_start.elapsed();
        tracing::debug!(
            "Warmup resample: {} -> {} samples in {:?}",
            audio.len(),
            resampled.len(),
            resample_elapsed
        );

        // Step 6: Convert to PCM16 bytes (warms up conversion code path)
        let pcm_start = std::time::Instant::now();
        let pcm_bytes = Self::f32_to_pcm16_warmup(&resampled);
        let pcm_elapsed = pcm_start.elapsed();
        tracing::debug!(
            "Warmup PCM16 conversion: {} samples -> {} bytes in {:?}",
            resampled.len(),
            pcm_bytes.len(),
            pcm_elapsed
        );

        let total_elapsed = start.elapsed();
        let duration_ms = audio.len() as f32 / KOKORO_SAMPLE_RATE as f32 * 1000.0;

        tracing::info!(
            "Kokoro TTS warmup completed: generated {:.0}ms of audio in {:?} (inference: {:?})",
            duration_ms,
            total_elapsed,
            inference_elapsed
        );

        Ok(())
    }

    /// Resample audio for warmup (same algorithm as KokoroTTS::resample)
    #[cfg(feature = "kokoro-tts")]
    fn resample_warmup(samples: &[f32], source_rate: u32, target_rate: u32) -> Vec<f32> {
        if source_rate == target_rate || samples.is_empty() {
            return samples.to_vec();
        }

        let ratio = source_rate as f64 / target_rate as f64;
        let output_len = ((samples.len() as f64) / ratio).ceil() as usize;
        let mut output = Vec::with_capacity(output_len);

        for i in 0..output_len {
            let src_pos = i as f64 * ratio;
            let src_idx = src_pos as usize;
            let frac = (src_pos - src_idx as f64) as f32;

            let sample = if src_idx + 1 < samples.len() {
                samples[src_idx] * (1.0 - frac) + samples[src_idx + 1] * frac
            } else if src_idx < samples.len() {
                samples[src_idx]
            } else {
                0.0
            };

            output.push(sample);
        }

        output
    }

    /// Convert f32 samples to PCM16 bytes for warmup
    #[cfg(feature = "kokoro-tts")]
    fn f32_to_pcm16_warmup(samples: &[f32]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(samples.len() * 2);
        for &sample in samples {
            let clamped = sample.clamp(-1.0, 1.0);
            let pcm16 = (clamped * 32767.0) as i16;
            bytes.extend_from_slice(&pcm16.to_le_bytes());
        }
        bytes
    }

    /// Stub for warmup() when feature is disabled
    #[cfg(not(feature = "kokoro-tts"))]
    pub async fn warmup(&self) -> TTSResult<()> {
        Ok(())
    }

    /// Load an additional voice into memory
    ///
    /// This is useful for preloading voices beyond the default.
    #[cfg(feature = "kokoro-tts")]
    pub fn load_voice(&self, voice_name: &str) -> TTSResult<VoiceManager> {
        let voice_path = assets::voice_path_sync(&self.asset_config, voice_name)
            .map_err(|e| TTSError::InvalidConfiguration(e.to_string()))?;

        VoiceManager::load_single(&voice_path, voice_name)
    }

    /// Stub for load_voice() when feature is disabled
    #[cfg(not(feature = "kokoro-tts"))]
    pub fn load_voice(&self, _voice_name: &str) -> TTSResult<VoiceManager> {
        Err(TTSError::InvalidConfiguration(
            "Kokoro TTS feature is not enabled".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_config_default() {
        let config = KokoroModelCacheConfig::default();
        assert_eq!(config.default_voice, "af_bella");
        assert_eq!(config.cache_path, PathBuf::from("/app/cache/kokoro"));
    }

    #[test]
    fn test_cache_config_custom() {
        let config = KokoroModelCacheConfig {
            cache_path: PathBuf::from("/custom/path"),
            default_voice: "am_adam".to_string(),
        };
        assert_eq!(config.default_voice, "am_adam");
        assert_eq!(config.cache_path, PathBuf::from("/custom/path"));
    }
}
