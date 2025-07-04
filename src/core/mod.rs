pub mod stt;
pub mod tts;

// Re-export commonly used types for convenience
pub use stt::{
    BaseSTT, DeepgramSTT, DeepgramSTTConfig, STTConfig, STTConnectionState, STTError, STTProvider,
    STTResult, STTResultCallback, STTStats, create_stt_provider, create_stt_provider_from_enum,
    get_supported_stt_providers,
};

pub use tts::{
    AudioCallback, AudioData, BaseTTS, BoxedTTS, ChannelAudioCallback, ConnectionState, TTSConfig,
    TTSError, TTSFactory, TTSResult,
};
