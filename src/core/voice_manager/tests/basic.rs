//! Basic VoiceManager tests for creation, configuration, and callback registration.
//!
//! Per CLAUDE.md "Testing" section, unit tests use `#[tokio::test]` for async tests.

use super::helpers::{create_voice_manager, default_voice_manager_config};

#[tokio::test]
async fn test_voice_manager_creation() {
    let config = default_voice_manager_config();
    let result = create_voice_manager!(config, None);
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_voice_manager_config_access() {
    let config = default_voice_manager_config();
    let voice_manager = create_voice_manager!(config, None).unwrap();
    let retrieved_config = voice_manager.get_config();

    assert_eq!(retrieved_config.stt_config.provider, "deepgram");
    assert_eq!(retrieved_config.tts_config.provider, "deepgram");
}

#[tokio::test]
async fn test_voice_manager_callback_registration() {
    let config = default_voice_manager_config();
    let voice_manager = create_voice_manager!(config, None).unwrap();

    // Test STT callback registration
    let stt_result = voice_manager
        .on_stt_result(|result| {
            Box::pin(async move {
                println!("STT Result: {}", result.transcript);
            })
        })
        .await;

    assert!(stt_result.is_ok());

    // Test TTS callback registration
    let tts_result = voice_manager
        .on_tts_audio(|audio_data| {
            Box::pin(async move {
                println!("TTS Audio: {} bytes", audio_data.data.len());
            })
        })
        .await;

    assert!(tts_result.is_ok());
}
