//! Kokoro TTS - Local ONNX-based Text-to-Speech Provider
//!
//! This module provides a local TTS solution using the Kokoro 82M neural model
//! from HuggingFace (onnx-community/Kokoro-82M-v1.0-ONNX-timestamped).
//!
//! Kokoro runs locally using ONNX Runtime and requires no cloud API.
//!
//! ## Features
//! - Local inference using ONNX Runtime
//! - 60 voices (American, British, and other languages)
//! - 24kHz mono audio output
//! - eSpeak-NG for text-to-phoneme conversion
//!
//! ## Usage
//!
//! ```rust,ignore
//! use sayna::core::tts::{create_tts_provider, TTSConfig};
//!
//! let config = TTSConfig {
//!     provider: "kokoro".to_string(),
//!     voice_id: Some("af_bella".to_string()),
//!     speaking_rate: Some(1.0),
//!     ..Default::default()
//! };
//!
//! let provider = create_tts_provider("kokoro", config)?;
//! ```

pub mod assets;
mod cache;
mod config;
mod model;
mod normalize;
mod phonemizer;
mod tokenizer;
mod vocab;
mod voice;

pub use assets::{KokoroAssetConfig, download_assets};
pub use cache::{KokoroModelCache, KokoroModelCacheConfig};
pub use config::KokoroConfig;

#[cfg(feature = "kokoro-tts")]
pub use model::ModelConfig;

use crate::core::tts::{
    AudioCallback, AudioData, BaseTTS, ConnectionState, TTSConfig, TTSError, TTSResult,
};
use async_trait::async_trait;
use std::sync::Arc;

use model::KokoroModel;
use phonemizer::Phonemizer;
use voice::VoiceManager;

/// Output sample rate for Kokoro TTS (24kHz)
pub const KOKORO_SAMPLE_RATE: u32 = 24000;

/// Default voice for Kokoro TTS
pub const DEFAULT_VOICE: &str = "af_bella";

/// Kokoro TTS Provider
///
/// A local ONNX-based TTS provider that runs the Kokoro 82M model.
pub struct KokoroTTS {
    config: KokoroConfig,
    model: Option<KokoroModel>,
    voice_manager: Option<VoiceManager>,
    phonemizer: Option<Phonemizer>,
    audio_callback: Option<Arc<dyn AudioCallback>>,
    connection_state: ConnectionState,
    /// Reference to preloaded model cache (if available)
    preloaded_cache: Option<Arc<KokoroModelCache>>,
}

impl KokoroTTS {
    /// Create a new KokoroTTS instance with a preloaded model cache
    ///
    /// This allows reusing the model and voice loaded during server startup,
    /// significantly reducing the time to first audio.
    ///
    /// # Arguments
    /// * `config` - TTS configuration
    /// * `cache` - Preloaded model cache from CoreState
    pub fn new_with_cache(config: TTSConfig, cache: Arc<KokoroModelCache>) -> TTSResult<Self> {
        let kokoro_config = KokoroConfig::from_tts_config(&config)?;

        Ok(Self {
            config: kokoro_config,
            model: None,
            voice_manager: None,
            phonemizer: None,
            audio_callback: None,
            connection_state: ConnectionState::Disconnected,
            preloaded_cache: Some(cache),
        })
    }

    /// Get the list of available voices
    pub fn get_available_voices(&self) -> Vec<String> {
        self.voice_manager
            .as_ref()
            .map(|vm| vm.get_available_voices())
            .unwrap_or_else(|| {
                // Fall back to listing from cache directory
                self.config
                    .to_asset_config()
                    .map(|ac| assets::list_available_voices(&ac))
                    .unwrap_or_default()
            })
    }

    /// Convert f32 audio samples to PCM16 bytes
    fn f32_to_pcm16_bytes(samples: &[f32]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(samples.len() * 2);
        for &sample in samples {
            let clamped = sample.clamp(-1.0, 1.0);
            let pcm16 = (clamped * 32767.0) as i16;
            bytes.extend_from_slice(&pcm16.to_le_bytes());
        }
        bytes
    }

    /// Resample audio from source rate to target rate using linear interpolation
    ///
    /// # Arguments
    /// * `samples` - Input audio samples at source_rate
    /// * `source_rate` - Source sample rate (e.g., 24000 for Kokoro native)
    /// * `target_rate` - Target sample rate
    ///
    /// # Returns
    /// Resampled audio at target_rate
    fn resample(samples: &[f32], source_rate: u32, target_rate: u32) -> Vec<f32> {
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
                // Linear interpolation between two samples
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

    /// Process text through the TTS pipeline
    async fn process_text(&mut self, text: &str) -> TTSResult<Vec<f32>> {
        // Get phonemizer - either local or from preloaded cache
        let phonemizer: &Phonemizer = if let Some(p) = self.phonemizer.as_ref() {
            p
        } else if let Some(cache) = &self.preloaded_cache {
            cache.phonemizer.as_ref()
        } else {
            return Err(TTSError::ProviderNotReady(
                "Phonemizer not initialized".to_string(),
            ));
        };

        // Get the voice name
        let voice_name = &self.config.voice_id;

        // Get language code from voice name (first char: 'a' = American, 'b' = British)
        let language = if voice_name.starts_with('b') {
            "b"
        } else {
            "a"
        };

        // Step 1: Normalize and phonemize the text
        let phonemes = phonemizer.phonemize(text, language);

        // Step 2: Tokenize the phonemes
        let tokens = tokenizer::tokenize(&phonemes);

        if tokens.is_empty() {
            return Ok(Vec::new());
        }

        // Step 3: Get voice style embedding
        // Use local voice_manager if available, otherwise use preloaded cache
        let style = if let Some(vm) = self.voice_manager.as_ref() {
            vm.get_style(voice_name, tokens.len())?
        } else if let Some(cache) = &self.preloaded_cache {
            // Check if we're using the default voice from cache
            if *voice_name == cache.default_voice_name {
                cache.default_voice.get_style(voice_name, tokens.len())?
            } else {
                return Err(TTSError::ProviderNotReady(format!(
                    "Voice '{}' not loaded. Expected '{}' or load a different voice.",
                    voice_name, cache.default_voice_name
                )));
            }
        } else {
            return Err(TTSError::ProviderNotReady(
                "Voice manager not initialized".to_string(),
            ));
        };

        // Step 4: Run inference
        // Use local model if available, otherwise use preloaded cache
        let audio = if let Some(model) = self.model.as_mut() {
            model
                .infer(tokens, style, self.config.speaking_rate)
                .await?
        } else if let Some(cache) = &self.preloaded_cache {
            // Use the preloaded model (requires unsafe due to Arc<KokoroModel>)
            // The model has internal Mutex, so concurrent access is safe
            let model_ptr = Arc::as_ptr(&cache.model) as *mut KokoroModel;
            let model_mut = unsafe { &mut *model_ptr };
            model_mut
                .infer(tokens, style, self.config.speaking_rate)
                .await?
        } else {
            return Err(TTSError::ProviderNotReady(
                "Model not initialized".to_string(),
            ));
        };

        Ok(audio)
    }
}

#[async_trait]
impl BaseTTS for KokoroTTS {
    fn new(config: TTSConfig) -> TTSResult<Self> {
        let kokoro_config = KokoroConfig::from_tts_config(&config)?;

        Ok(Self {
            config: kokoro_config,
            model: None,
            voice_manager: None,
            phonemizer: None,
            audio_callback: None,
            connection_state: ConnectionState::Disconnected,
            preloaded_cache: None,
        })
    }

    async fn connect(&mut self) -> TTSResult<()> {
        self.connection_state = ConnectionState::Connecting;

        // Check if we can use preloaded resources
        if let Some(cache) = &self.preloaded_cache {
            // Use preloaded phonemizer (clone the Arc reference)
            // Note: Phonemizer is stateless and uses global mutex internally
            let phonemizer = Phonemizer::new()?;
            self.phonemizer = Some(phonemizer);

            // Check if the requested voice matches the preloaded default voice
            if self.config.voice_id == cache.default_voice_name {
                tracing::info!(
                    "Kokoro TTS using preloaded model and voice '{}'",
                    cache.default_voice_name
                );
                // We don't copy the model/voice_manager since they're behind Arc
                // Instead, we'll use them directly in process_text via the cache
                self.connection_state = ConnectionState::Connected;
                return Ok(());
            } else {
                // Different voice requested - need to load it
                tracing::info!(
                    "Kokoro TTS using preloaded model, loading voice '{}'",
                    self.config.voice_id
                );
                self.voice_manager = Some(cache.load_voice(&self.config.voice_id)?);
                self.connection_state = ConnectionState::Connected;
                return Ok(());
            }
        }

        // No preloaded cache - load everything from scratch
        tracing::debug!("Kokoro TTS loading model from scratch (no preloaded cache)");

        // Initialize the phonemizer
        self.phonemizer = Some(Phonemizer::new()?);

        // Get asset configuration
        let asset_config = self.config.to_asset_config()?;

        // Load model
        let model_path = assets::model_path(&asset_config)
            .map_err(|e| TTSError::InvalidConfiguration(e.to_string()))?;

        self.model = Some(KokoroModel::load(&model_path).await?);

        // Load voice
        let voice_path = assets::voice_path_sync(&asset_config, &self.config.voice_id)
            .map_err(|e| TTSError::InvalidConfiguration(e.to_string()))?;

        self.voice_manager = Some(VoiceManager::load_single(
            &voice_path,
            &self.config.voice_id,
        )?);

        self.connection_state = ConnectionState::Connected;
        tracing::info!(
            "Kokoro TTS connected: model={:?}, voice={}",
            model_path,
            self.config.voice_id
        );

        Ok(())
    }

    async fn disconnect(&mut self) -> TTSResult<()> {
        self.model = None;
        self.voice_manager = None;
        self.phonemizer = None;
        self.connection_state = ConnectionState::Disconnected;
        tracing::info!("Kokoro TTS disconnected");
        Ok(())
    }

    fn is_ready(&self) -> bool {
        if !matches!(self.connection_state, ConnectionState::Connected) {
            return false;
        }

        // Check if we have local resources
        let has_local_resources =
            self.model.is_some() && self.phonemizer.is_some() && self.voice_manager.is_some();

        // Check if we can use preloaded cache (for default voice)
        let can_use_preloaded_cache = if let Some(cache) = &self.preloaded_cache {
            // Using preloaded cache with default voice OR with custom loaded voice_manager
            let using_default_voice = self.config.voice_id == cache.default_voice_name;
            let has_phonemizer = self.phonemizer.is_some();

            // Either: default voice with phonemizer, or custom voice with voice_manager + phonemizer
            (self.voice_manager.is_some() || using_default_voice) && has_phonemizer
        } else {
            false
        };

        has_local_resources || can_use_preloaded_cache
    }

    fn get_connection_state(&self) -> ConnectionState {
        self.connection_state.clone()
    }

    async fn speak(&mut self, text: &str, flush: bool) -> TTSResult<()> {
        if !self.is_ready() {
            self.connect().await?;
        }

        self.connection_state = ConnectionState::Processing;

        // Process the text (generates audio at native 24kHz)
        let audio_samples = self.process_text(text).await?;

        if audio_samples.is_empty() {
            self.connection_state = ConnectionState::Connected;
            return Ok(());
        }

        // Resample to configured sample rate if different from native rate
        let target_rate = self.config.sample_rate;
        let final_samples = if target_rate != KOKORO_SAMPLE_RATE {
            tracing::debug!(
                "Resampling from {}Hz to {}Hz",
                KOKORO_SAMPLE_RATE,
                target_rate
            );
            Self::resample(&audio_samples, KOKORO_SAMPLE_RATE, target_rate)
        } else {
            audio_samples
        };

        // Convert to PCM16 bytes
        let audio_bytes = Self::f32_to_pcm16_bytes(&final_samples);

        // Calculate duration based on final sample rate
        let duration_ms = (final_samples.len() as f32 / target_rate as f32 * 1000.0) as u32;

        // Create audio data with configured sample rate
        let audio_data = AudioData {
            data: audio_bytes,
            sample_rate: target_rate,
            format: "linear16".to_string(),
            duration_ms: Some(duration_ms),
        };

        // Send to callback
        if let Some(callback) = &self.audio_callback {
            callback.on_audio(audio_data).await;
            if flush {
                callback.on_complete().await;
            }
        }

        self.connection_state = ConnectionState::Connected;
        Ok(())
    }

    async fn clear(&mut self) -> TTSResult<()> {
        // Kokoro processes synchronously, so nothing to clear
        Ok(())
    }

    async fn flush(&self) -> TTSResult<()> {
        if let Some(callback) = &self.audio_callback {
            callback.on_complete().await;
        }
        Ok(())
    }

    fn on_audio(&mut self, callback: Arc<dyn AudioCallback>) -> TTSResult<()> {
        self.audio_callback = Some(callback);
        Ok(())
    }

    fn remove_audio_callback(&mut self) -> TTSResult<()> {
        self.audio_callback = None;
        Ok(())
    }

    fn get_provider_info(&self) -> serde_json::Value {
        serde_json::json!({
            "provider": "kokoro",
            "version": "1.0.0",
            "model": "kokoro-82m-quantized",
            "model_source": "huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX-timestamped",
            "sample_rate": KOKORO_SAMPLE_RATE,
            "audio_format": "linear16",
            "available_voices": self.get_available_voices(),
            "local": true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_f32_to_pcm16() {
        let samples = vec![0.0, 0.5, -0.5, 1.0, -1.0];
        let bytes = KokoroTTS::f32_to_pcm16_bytes(&samples);

        assert_eq!(bytes.len(), samples.len() * 2);

        // Check 0.0 -> 0
        assert_eq!(i16::from_le_bytes([bytes[0], bytes[1]]), 0);

        // Check 0.5 -> ~16383
        let val = i16::from_le_bytes([bytes[2], bytes[3]]);
        assert!((val - 16383).abs() < 2);

        // Check -0.5 -> ~-16383
        let val = i16::from_le_bytes([bytes[4], bytes[5]]);
        assert!((val + 16383).abs() < 2);

        // Check 1.0 -> 32767
        assert_eq!(i16::from_le_bytes([bytes[6], bytes[7]]), 32767);

        // Check -1.0 -> -32767
        assert_eq!(i16::from_le_bytes([bytes[8], bytes[9]]), -32767);
    }

    #[test]
    fn test_resample_same_rate() {
        let samples = vec![0.0, 0.5, 1.0, 0.5, 0.0];
        let result = KokoroTTS::resample(&samples, 24000, 24000);
        assert_eq!(result, samples);
    }

    #[test]
    fn test_resample_empty() {
        let samples: Vec<f32> = vec![];
        let result = KokoroTTS::resample(&samples, 24000, 16000);
        assert!(result.is_empty());
    }

    #[test]
    fn test_resample_downsample() {
        // 24kHz to 16kHz = 2/3 ratio, so 6 samples -> 4 samples
        let samples = vec![0.0, 0.3, 0.6, 0.9, 0.6, 0.3];
        let result = KokoroTTS::resample(&samples, 24000, 16000);
        assert_eq!(result.len(), 4);
        // First sample should be 0.0
        assert!((result[0] - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_resample_upsample() {
        // 24kHz to 48kHz = 1/2 ratio, so 4 samples -> 8 samples
        let samples = vec![0.0, 0.5, 1.0, 0.5];
        let result = KokoroTTS::resample(&samples, 24000, 48000);
        assert_eq!(result.len(), 8);
        // First sample should be 0.0
        assert!((result[0] - 0.0).abs() < 0.01);
        // Sample at position 2 should be ~0.5 (interpolated between 0.0 and 0.5)
        assert!((result[1] - 0.25).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_new_kokoro_tts() {
        let config = TTSConfig {
            provider: "kokoro".to_string(),
            voice_id: Some("af_bella".to_string()),
            ..Default::default()
        };

        // Should create without error (but not be ready until connected)
        let result = KokoroTTS::new(config);
        assert!(result.is_ok());

        let tts = result.unwrap();
        assert!(!tts.is_ready());
        assert_eq!(tts.get_connection_state(), ConnectionState::Disconnected);
    }

    #[test]
    fn test_default_voice_constant() {
        assert_eq!(assets::DEFAULT_VOICE, "af_bella");
    }
}
