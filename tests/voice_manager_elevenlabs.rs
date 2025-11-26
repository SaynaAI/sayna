//! VoiceManager integration tests with ElevenLabs STT
//!
//! These tests verify that ElevenLabs STT works correctly
//! when used through the VoiceManager abstraction.

use sayna::core::stt::STTConfig;
use sayna::core::tts::TTSConfig;
use sayna::core::voice_manager::{VoiceManager, VoiceManagerConfig};

/// Test VoiceManager creation with ElevenLabs STT
#[test]
fn test_voice_manager_with_elevenlabs_stt() {
    let stt_config = STTConfig {
        provider: "elevenlabs".to_string(),
        api_key: "test-stt-key".to_string(),
        language: "en".to_string(),
        sample_rate: 16000,
        channels: 1,
        punctuation: true,
        encoding: "linear16".to_string(),
        model: "".to_string(),
    };

    let tts_config = TTSConfig {
        provider: "elevenlabs".to_string(),
        api_key: "test-tts-key".to_string(),
        voice_id: Some("test-voice".to_string()),
        ..Default::default()
    };

    let config = VoiceManagerConfig::new(stt_config, tts_config);
    let result = VoiceManager::new(config, None);

    assert!(result.is_ok());
}

/// Test VoiceManager fails gracefully with empty API key
#[test]
fn test_voice_manager_elevenlabs_requires_api_key() {
    let stt_config = STTConfig {
        provider: "elevenlabs".to_string(),
        api_key: String::new(), // Empty key
        language: "en".to_string(),
        sample_rate: 16000,
        channels: 1,
        punctuation: true,
        encoding: "linear16".to_string(),
        model: "".to_string(),
    };

    let tts_config = TTSConfig::default();

    let config = VoiceManagerConfig::new(stt_config, tts_config);
    let result = VoiceManager::new(config, None);

    assert!(result.is_err());
}

/// Test VoiceManager with ElevenLabs STT and Deepgram TTS
#[test]
fn test_voice_manager_elevenlabs_stt_deepgram_tts() {
    let stt_config = STTConfig {
        provider: "elevenlabs".to_string(),
        api_key: "test-elevenlabs-key".to_string(),
        language: "en".to_string(),
        sample_rate: 16000,
        channels: 1,
        punctuation: true,
        encoding: "linear16".to_string(),
        model: "".to_string(),
    };

    let tts_config = TTSConfig {
        provider: "deepgram".to_string(),
        api_key: "test-deepgram-key".to_string(),
        voice_id: Some("aura-asteria-en".to_string()),
        ..Default::default()
    };

    let config = VoiceManagerConfig::new(stt_config, tts_config);
    let result = VoiceManager::new(config, None);

    assert!(result.is_ok());
}

/// Test VoiceManager with different sample rates
#[test]
fn test_voice_manager_elevenlabs_different_sample_rates() {
    let sample_rates = vec![8000, 16000, 22050, 24000, 44100, 48000];

    for sample_rate in sample_rates {
        let stt_config = STTConfig {
            provider: "elevenlabs".to_string(),
            api_key: "test-key".to_string(),
            language: "en".to_string(),
            sample_rate,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            model: "".to_string(),
        };

        let tts_config = TTSConfig {
            provider: "elevenlabs".to_string(),
            api_key: "test-key".to_string(),
            ..Default::default()
        };

        let config = VoiceManagerConfig::new(stt_config, tts_config);
        let result = VoiceManager::new(config, None);

        assert!(
            result.is_ok(),
            "VoiceManager failed with sample rate: {}",
            sample_rate
        );
    }
}

/// Test VoiceManager with different languages
#[test]
fn test_voice_manager_elevenlabs_different_languages() {
    let languages = vec!["en", "es", "fr", "de", "it", "pt", "ja", "ko", "zh"];

    for language in languages {
        let stt_config = STTConfig {
            provider: "elevenlabs".to_string(),
            api_key: "test-key".to_string(),
            language: language.to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            model: "".to_string(),
        };

        let tts_config = TTSConfig {
            provider: "elevenlabs".to_string(),
            api_key: "test-key".to_string(),
            ..Default::default()
        };

        let config = VoiceManagerConfig::new(stt_config, tts_config);
        let result = VoiceManager::new(config, None);

        assert!(
            result.is_ok(),
            "VoiceManager failed with language: {}",
            language
        );
    }
}
