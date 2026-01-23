//! Shared test helpers for VoiceManager tests.
//!
//! Per CLAUDE.md "Testing" section, shared helpers reduce duplication
//! and improve test maintainability.

use crate::core::stt::STTConfig;
use crate::core::tts::TTSConfig;
use crate::core::voice_manager::VoiceManagerConfig;

/// Creates a default STTConfig for testing with Deepgram provider.
pub fn default_stt_config() -> STTConfig {
    STTConfig {
        provider: "deepgram".to_string(),
        api_key: "test_key".to_string(),
        ..Default::default()
    }
}

/// Creates a default TTSConfig for testing with Deepgram provider.
pub fn default_tts_config() -> TTSConfig {
    TTSConfig {
        provider: "deepgram".to_string(),
        api_key: "test_key".to_string(),
        ..Default::default()
    }
}

/// Creates a VoiceManagerConfig with default STT and TTS configs.
pub fn default_voice_manager_config() -> VoiceManagerConfig {
    VoiceManagerConfig::new(default_stt_config(), default_tts_config())
}

/// Creates a stub STTConfig for testing with custom providers.
/// Only used when stt-vad feature is enabled.
#[cfg(feature = "stt-vad")]
pub fn stub_stt_config() -> STTConfig {
    STTConfig {
        provider: "stub".to_string(),
        api_key: "test".to_string(),
        ..Default::default()
    }
}

/// Creates a stub TTSConfig for testing with custom providers.
/// Only used when stt-vad feature is enabled.
#[cfg(feature = "stt-vad")]
pub fn stub_tts_config() -> TTSConfig {
    TTSConfig {
        provider: "stub".to_string(),
        api_key: "test".to_string(),
        ..Default::default()
    }
}

/// Creates a VoiceManagerConfig with stub STT and TTS configs.
/// Only used when stt-vad feature is enabled.
#[cfg(feature = "stt-vad")]
pub fn stub_voice_manager_config() -> VoiceManagerConfig {
    VoiceManagerConfig::new(stub_stt_config(), stub_tts_config())
}

/// Helper macro to create VoiceManager with feature-gated VAD parameter.
#[cfg(feature = "stt-vad")]
#[macro_export]
macro_rules! create_voice_manager {
    ($config:expr, $turn_detector:expr) => {
        $crate::core::voice_manager::VoiceManager::new($config, $turn_detector, None)
    };
}

#[cfg(not(feature = "stt-vad"))]
#[macro_export]
macro_rules! create_voice_manager {
    ($config:expr, $turn_detector:expr) => {
        $crate::core::voice_manager::VoiceManager::new($config, $turn_detector)
    };
}

// Re-export the macro for use in test modules
pub use create_voice_manager;
