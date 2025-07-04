pub mod stt;
pub mod tts;

// Re-export commonly used types for convenience
pub use tts::{
    AudioCallback, AudioData, BaseTTS, BoxedTTS, ChannelAudioCallback, ConnectionState, TTSConfig,
    TTSError, TTSFactory, TTSResult,
};
