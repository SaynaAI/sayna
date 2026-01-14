//! WebSocket configuration types and handlers
//!
//! This module contains all configuration-related types for WebSocket connections,
//! including STT, TTS, and LiveKit configurations without API keys.

use serde::{Deserialize, Serialize};
use xxhash_rust::xxh3::xxh3_128;

use crate::{
    core::{
        stt::STTConfig,
        tts::{Pronunciation, TTSConfig},
        vad::{SileroVADConfig, VADSilenceConfig},
    },
    livekit::LiveKitConfig,
};

/// Default value for audio enabled flag (true)
pub fn default_audio_enabled() -> Option<bool> {
    Some(true)
}

/// Default value for allow_interruption flag (true)
pub fn default_allow_interruption() -> Option<bool> {
    Some(true)
}

/// Default VAD threshold (0.5)
fn default_vad_threshold() -> f32 {
    0.5
}

/// Default silence duration in milliseconds (300)
fn default_silence_duration_ms() -> u64 {
    300
}

/// Default minimum speech duration in milliseconds (100)
fn default_min_speech_duration_ms() -> u64 {
    100
}

/// VAD configuration for WebSocket messages
///
/// Voice Activity Detection (VAD) can be used to detect when a speaker has
/// finished talking, based on silence duration thresholds. This provides
/// an alternative or supplement to STT provider-based speech final detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct VADWebSocketConfig {
    /// Whether to enable VAD-based silence detection.
    ///
    /// When enabled, VAD results are used to determine when a speaker
    /// has finished talking, based on silence duration thresholds.
    /// Default: false (for backward compatibility).
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(example = true))]
    pub enabled: bool,

    /// Speech probability threshold (0.0 to 1.0).
    ///
    /// Audio frames with probability above this threshold are considered speech.
    /// The default value of 0.5 is the recommendation from Silero-VAD.
    ///
    /// Lower values increase sensitivity (detect more speech, more false positives).
    /// Higher values decrease sensitivity (miss quiet speech, fewer false positives).
    #[serde(default = "default_vad_threshold")]
    #[cfg_attr(feature = "openapi", schema(example = 0.5))]
    pub threshold: f32,

    /// Silence duration in milliseconds to trigger turn end.
    ///
    /// After detecting speech, this is how long continuous silence must be
    /// observed before considering the speech segment complete. Default: 300ms.
    #[serde(default = "default_silence_duration_ms")]
    #[cfg_attr(feature = "openapi", schema(example = 300))]
    pub silence_duration_ms: u64,

    /// Minimum speech duration in milliseconds before considering silence.
    ///
    /// Prevents false triggers on brief pauses within speech. The system won't
    /// start tracking silence until at least this much speech has been detected.
    /// Default: 100ms.
    #[serde(default = "default_min_speech_duration_ms")]
    #[cfg_attr(feature = "openapi", schema(example = 100))]
    pub min_speech_duration_ms: u64,
}

impl Default for VADWebSocketConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold: default_vad_threshold(),
            silence_duration_ms: default_silence_duration_ms(),
            min_speech_duration_ms: default_min_speech_duration_ms(),
        }
    }
}

impl VADWebSocketConfig {
    /// Convert WebSocket VAD config to VADSilenceConfig for VoiceManager
    ///
    /// This creates a VADSilenceConfig with the WebSocket-provided settings,
    /// using default values for Silero-VAD model configuration (model path, etc.)
    /// which are handled by the server configuration.
    pub fn to_vad_silence_config(&self) -> VADSilenceConfig {
        let silero_config = SileroVADConfig {
            threshold: self.threshold,
            silence_duration_ms: self.silence_duration_ms,
            min_speech_duration_ms: self.min_speech_duration_ms,
            ..SileroVADConfig::default()
        };

        VADSilenceConfig {
            enabled: self.enabled,
            silero_config,
        }
    }
}

/// STT configuration for WebSocket messages (without API key)
#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct STTWebSocketConfig {
    /// Provider name (e.g., "deepgram")
    #[cfg_attr(feature = "openapi", schema(example = "deepgram"))]
    pub provider: String,
    /// Language code for transcription (e.g., "en-US", "es-ES")
    #[cfg_attr(feature = "openapi", schema(example = "en-US"))]
    pub language: String,
    /// Sample rate of the audio in Hz
    #[cfg_attr(feature = "openapi", schema(example = 16000))]
    pub sample_rate: u32,
    /// Number of audio channels (1 for mono, 2 for stereo)
    #[cfg_attr(feature = "openapi", schema(example = 1))]
    pub channels: u16,
    /// Enable punctuation in results
    #[cfg_attr(feature = "openapi", schema(example = true))]
    pub punctuation: bool,
    /// Encoding of the audio
    #[cfg_attr(feature = "openapi", schema(example = "linear16"))]
    pub encoding: String,
    /// Model to use for transcription
    #[cfg_attr(feature = "openapi", schema(example = "nova-2"))]
    pub model: String,
}

impl STTWebSocketConfig {
    /// Convert WebSocket STT config to full STT config with API key
    ///
    /// # Arguments
    /// * `api_key` - The API key to use for this provider
    ///
    /// # Returns
    /// * `STTConfig` - Full STT configuration
    pub fn to_stt_config(&self, api_key: String) -> STTConfig {
        STTConfig {
            provider: self.provider.clone(),
            api_key,
            language: self.language.clone(),
            sample_rate: self.sample_rate,
            channels: self.channels,
            punctuation: self.punctuation,
            encoding: self.encoding.clone(),
            model: self.model.clone(),
        }
    }
}

/// LiveKit configuration for WebSocket messages
#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LiveKitWebSocketConfig {
    /// Room name to join or create
    #[cfg_attr(feature = "openapi", schema(example = "conversation-room-123"))]
    pub room_name: String,
    /// Enable recording for this session
    #[serde(default)]
    pub enable_recording: bool,
    // recording_file_key removed; recording path now determined by stream_id + server prefix
    /// Sayna AI participant identity (defaults to "sayna-ai")
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "sayna-ai"))]
    pub sayna_participant_identity: Option<String>,
    /// Sayna AI participant display name (defaults to "Sayna AI")
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "Sayna AI"))]
    pub sayna_participant_name: Option<String>,
    /// List of participant identities to listen to for audio tracks and data messages. (All participants by default)
    ///
    /// **Behavior**:
    /// - If **empty** (default): Audio tracks and data messages from **all participants** will be processed
    /// - If **populated**: Only audio tracks and data messages from participants whose identities
    ///   are in this list will be processed; others will be ignored
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub listen_participants: Vec<String>,
}

impl LiveKitWebSocketConfig {
    /// Convert WebSocket LiveKit config to full LiveKit config with audio parameters
    ///
    /// # Arguments
    /// * `token` - JWT token for LiveKit room access (generated by LiveKitRoomHandler)
    /// * `tts_config` - TTS configuration containing audio parameters
    /// * `livekit_url` - LiveKit server URL
    ///
    /// # Returns
    /// * `LiveKitConfig` - Full LiveKit configuration with audio parameters
    pub fn to_livekit_config(
        &self,
        token: String,
        tts_config: &TTSWebSocketConfig,
        livekit_url: &str,
    ) -> LiveKitConfig {
        LiveKitConfig {
            url: livekit_url.to_string(),
            token,
            room_name: self.room_name.clone(),
            // Use TTS config sample rate, default to 24000 if not specified
            sample_rate: tts_config.sample_rate.unwrap_or(24000),
            // Assume mono audio for TTS (1 channel)
            channels: 1,
            // Enable noise filter by default when compiled with the optional feature
            // Can be disabled via config if lower latency is needed
            enable_noise_filter: cfg!(feature = "noise-filter"),
            // Pass through the participant filter list
            listen_participants: self.listen_participants.clone(),
        }
    }
}

/// TTS configuration for WebSocket messages (without API key)
#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TTSWebSocketConfig {
    /// Provider name (e.g., "deepgram")
    #[cfg_attr(feature = "openapi", schema(example = "deepgram"))]
    pub provider: String,
    /// Voice ID or name to use for synthesis
    #[cfg_attr(feature = "openapi", schema(example = "aura-asteria-en"))]
    pub voice_id: Option<String>,
    /// Speaking rate (0.25 to 4.0, 1.0 is normal)
    #[cfg_attr(feature = "openapi", schema(example = 1.0))]
    pub speaking_rate: Option<f32>,
    /// Audio format preference
    #[cfg_attr(feature = "openapi", schema(example = "linear16"))]
    pub audio_format: Option<String>,
    /// Sample rate preference
    #[cfg_attr(feature = "openapi", schema(example = 24000))]
    pub sample_rate: Option<u32>,
    /// Connection timeout in seconds
    #[cfg_attr(feature = "openapi", schema(example = 30))]
    pub connection_timeout: Option<u64>,
    /// Request timeout in seconds
    #[cfg_attr(feature = "openapi", schema(example = 60))]
    pub request_timeout: Option<u64>,
    /// Model to use for TTS
    #[cfg_attr(feature = "openapi", schema(example = "aura-asteria-en"))]
    pub model: String,
    /// Pronunciation replacements to apply before TTS
    #[serde(default)]
    pub pronunciations: Vec<Pronunciation>,
}

impl TTSWebSocketConfig {
    /// Convert WebSocket TTS config to full TTS config with API key and proper defaults
    ///
    /// # Arguments
    /// * `api_key` - The API key to use for this provider
    ///
    /// # Returns
    /// * `TTSConfig` - Full TTS configuration with defaults applied
    pub fn to_tts_config(&self, api_key: String) -> TTSConfig {
        // Start with defaults
        let defaults = TTSConfig::default();

        TTSConfig {
            provider: self.provider.clone(),
            api_key,
            model: self.model.clone(),
            // Use provided values or fall back to defaults
            voice_id: self.voice_id.clone().or(defaults.voice_id),
            speaking_rate: self.speaking_rate.or(defaults.speaking_rate),
            audio_format: self.audio_format.clone().or(defaults.audio_format),
            sample_rate: self.sample_rate.or(defaults.sample_rate),
            connection_timeout: self.connection_timeout.or(defaults.connection_timeout),
            request_timeout: self.request_timeout.or(defaults.request_timeout),
            pronunciations: self.pronunciations.clone(),
            request_pool_size: defaults.request_pool_size,
        }
    }
}

/// Compute TTS configuration hash for caching
pub fn compute_tts_config_hash(tts_config: &TTSConfig) -> String {
    let mut s = String::new();
    s.push_str(tts_config.provider.as_str());
    s.push('|');
    s.push_str(tts_config.voice_id.as_deref().unwrap_or(""));
    s.push('|');
    s.push_str(&tts_config.model);
    s.push('|');
    s.push_str(tts_config.audio_format.as_deref().unwrap_or(""));
    s.push('|');
    if let Some(sr) = tts_config.sample_rate {
        s.push_str(&sr.to_string());
    }
    s.push('|');
    if let Some(rate) = tts_config.speaking_rate {
        s.push_str(&format!("{rate:.3}"));
    }
    format!("{:032x}", xxh3_128(s.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vad_websocket_config_defaults() {
        let config = VADWebSocketConfig::default();

        assert!(!config.enabled);
        assert_eq!(config.threshold, 0.5);
        assert_eq!(config.silence_duration_ms, 300);
        assert_eq!(config.min_speech_duration_ms, 100);
    }

    #[test]
    fn test_vad_websocket_config_serialization() {
        let config = VADWebSocketConfig {
            enabled: true,
            threshold: 0.6,
            silence_duration_ms: 500,
            min_speech_duration_ms: 150,
        };

        let json = serde_json::to_string(&config).expect("Should serialize");
        assert!(json.contains("\"enabled\":true"));
        assert!(json.contains("\"threshold\":0.6"));
        assert!(json.contains("\"silence_duration_ms\":500"));
        assert!(json.contains("\"min_speech_duration_ms\":150"));
    }

    #[test]
    fn test_vad_websocket_config_deserialization() {
        let json = r#"{"enabled": true, "threshold": 0.7, "silence_duration_ms": 400}"#;
        let config: VADWebSocketConfig = serde_json::from_str(json).expect("Should deserialize");

        assert!(config.enabled);
        assert_eq!(config.threshold, 0.7);
        assert_eq!(config.silence_duration_ms, 400);
        // min_speech_duration_ms should have default value
        assert_eq!(config.min_speech_duration_ms, 100);
    }

    #[test]
    fn test_vad_websocket_config_deserialization_minimal() {
        // Test with only enabled field - all others should get defaults
        let json = r#"{"enabled": true}"#;
        let config: VADWebSocketConfig = serde_json::from_str(json).expect("Should deserialize");

        assert!(config.enabled);
        assert_eq!(config.threshold, 0.5);
        assert_eq!(config.silence_duration_ms, 300);
        assert_eq!(config.min_speech_duration_ms, 100);
    }

    #[test]
    fn test_vad_websocket_config_deserialization_empty() {
        // Test with empty object - should get all defaults including enabled=false
        let json = r#"{}"#;
        let config: VADWebSocketConfig = serde_json::from_str(json).expect("Should deserialize");

        assert!(!config.enabled);
        assert_eq!(config.threshold, 0.5);
        assert_eq!(config.silence_duration_ms, 300);
        assert_eq!(config.min_speech_duration_ms, 100);
    }

    #[test]
    fn test_vad_websocket_config_to_vad_silence_config() {
        let ws_config = VADWebSocketConfig {
            enabled: true,
            threshold: 0.65,
            silence_duration_ms: 450,
            min_speech_duration_ms: 200,
        };

        let vad_config = ws_config.to_vad_silence_config();

        assert!(vad_config.enabled);
        assert_eq!(vad_config.silero_config.threshold, 0.65);
        assert_eq!(vad_config.silero_config.silence_duration_ms, 450);
        assert_eq!(vad_config.silero_config.min_speech_duration_ms, 200);
    }

    #[test]
    fn test_vad_websocket_config_disabled_conversion() {
        let ws_config = VADWebSocketConfig::default();

        let vad_config = ws_config.to_vad_silence_config();

        assert!(!vad_config.enabled);
    }
}
