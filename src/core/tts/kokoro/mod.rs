//! Kokoro TTS - Local ONNX-based Text-to-Speech Provider
//!
//! This module provides a local TTS solution using the Kokoro 82M neural model.
//! Kokoro runs locally using ONNX Runtime and requires no cloud API.
//!
//! ## Features
//! - Local inference using ONNX Runtime
//! - 24 English voices (American and British)
//! - 24kHz mono audio output
//! - eSpeak-NG for text-to-phoneme conversion
//! - Optional word-level timestamps
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
mod config;
mod model;
mod normalize;
mod phonemizer;
mod tokenizer;
mod vocab;
mod voice;

pub use assets::{KokoroAssetConfig, download_assets};
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
}

impl KokoroTTS {
    /// Get the list of available voices
    pub fn get_available_voices(&self) -> Vec<String> {
        self.voice_manager
            .as_ref()
            .map(|vm| vm.get_available_voices())
            .unwrap_or_default()
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

    /// Process text through the TTS pipeline
    async fn process_text(&mut self, text: &str) -> TTSResult<Vec<f32>> {
        let phonemizer = self
            .phonemizer
            .as_ref()
            .ok_or_else(|| TTSError::ProviderNotReady("Phonemizer not initialized".to_string()))?;

        let model = self
            .model
            .as_mut()
            .ok_or_else(|| TTSError::ProviderNotReady("Model not initialized".to_string()))?;

        let voice_manager = self.voice_manager.as_ref().ok_or_else(|| {
            TTSError::ProviderNotReady("Voice manager not initialized".to_string())
        })?;

        // Get the voice name
        let voice_name = self.config.voice_id.as_deref().unwrap_or(DEFAULT_VOICE);

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
        let style = voice_manager.get_style(voice_name, tokens.len())?;

        // Step 4: Run inference
        let speed = self.config.speaking_rate.unwrap_or(1.0);
        let audio = model.infer(tokens, style, speed).await?;

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
        })
    }

    async fn connect(&mut self) -> TTSResult<()> {
        self.connection_state = ConnectionState::Connecting;

        // Initialize the phonemizer
        self.phonemizer = Some(Phonemizer::new()?);

        // Resolve model path using asset system
        // If explicit path is configured and exists, use it; otherwise use cache path resolution
        let model_path = if let Some(ref explicit_path) = self.config.model_path {
            if explicit_path.exists() {
                explicit_path.clone()
            } else {
                return Err(TTSError::InvalidConfiguration(format!(
                    "Model file not found at configured path: {}. Run 'sayna init' to download.",
                    explicit_path.display()
                )));
            }
        } else {
            // Use asset path resolution (checks cache, returns error if missing)
            let asset_config = self.config.to_asset_config();
            assets::model_path(&asset_config)
                .map_err(|e| TTSError::InvalidConfiguration(e.to_string()))?
        };

        self.model = Some(KokoroModel::load(&model_path).await?);

        // Resolve voices path using asset system
        let voices_path = if let Some(ref explicit_path) = self.config.voices_path {
            if explicit_path.exists() {
                explicit_path.clone()
            } else {
                return Err(TTSError::InvalidConfiguration(format!(
                    "Voices file not found at configured path: {}. Run 'sayna init' to download.",
                    explicit_path.display()
                )));
            }
        } else {
            // Use asset path resolution (checks cache, returns error if missing)
            let asset_config = self.config.to_asset_config();
            assets::voices_path(&asset_config)
                .map_err(|e| TTSError::InvalidConfiguration(e.to_string()))?
        };

        self.voice_manager = Some(VoiceManager::load(&voices_path)?);

        self.connection_state = ConnectionState::Connected;
        tracing::info!(
            "Kokoro TTS connected: model={:?}, voices={:?}",
            model_path,
            voices_path
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
        matches!(self.connection_state, ConnectionState::Connected)
            && self.model.is_some()
            && self.phonemizer.is_some()
            && self.voice_manager.is_some()
    }

    fn get_connection_state(&self) -> ConnectionState {
        self.connection_state.clone()
    }

    async fn speak(&mut self, text: &str, flush: bool) -> TTSResult<()> {
        if !self.is_ready() {
            self.connect().await?;
        }

        self.connection_state = ConnectionState::Processing;

        // Process the text
        let audio_samples = self.process_text(text).await?;

        if audio_samples.is_empty() {
            self.connection_state = ConnectionState::Connected;
            return Ok(());
        }

        // Convert to PCM16 bytes
        let audio_bytes = Self::f32_to_pcm16_bytes(&audio_samples);

        // Calculate duration
        let duration_ms = (audio_samples.len() as f32 / KOKORO_SAMPLE_RATE as f32 * 1000.0) as u32;

        // Create audio data
        let audio_data = AudioData {
            data: audio_bytes,
            sample_rate: KOKORO_SAMPLE_RATE,
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
            "model": "kokoro-82m",
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
}
