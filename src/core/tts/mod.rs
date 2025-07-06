mod base;
pub mod deepgram;

pub use base::{
    AudioCallback, AudioData, BaseTTS, BoxedTTS, ChannelAudioCallback, ConnectionState, TTSConfig,
    TTSError, TTSFactory, TTSResult,
};
pub use deepgram::DeepgramTTS;

/// Factory function to create a TTS provider
pub fn create_tts_provider(provider_type: &str, config: TTSConfig) -> TTSResult<Box<dyn BaseTTS>> {
    match provider_type.to_lowercase().as_str() {
        "deepgram" => Ok(Box::new(DeepgramTTS::new(config)?)),
        _ => Err(TTSError::InvalidConfiguration(format!(
            "Unsupported TTS provider: {provider_type}. Supported providers: deepgram"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_tts_provider() {
        let config = TTSConfig::default();
        let result = create_tts_provider("deepgram", config);
        assert!(result.is_ok());

        let invalid_result = create_tts_provider("invalid", TTSConfig::default());
        assert!(invalid_result.is_err());
    }
}
