pub mod cache;
pub mod noise_filter;
pub mod providers;
pub mod state;
pub mod stt;
pub mod tts;
pub mod turn_detect;
pub mod vad;
pub mod voice_manager;

#[cfg(feature = "stt-vad")]
pub use turn_detect::{TurnDetector, TurnDetectorBuilder, TurnDetectorConfig};
#[cfg(not(feature = "stt-vad"))]
pub use turn_detect::{TurnDetector, TurnDetectorBuilder, TurnDetectorConfig};

// Re-export VAD types
pub use vad::{SilenceTracker, SileroVAD, SileroVADConfig};

// Re-export commonly used types for convenience
pub use stt::{
    BaseSTT, DeepgramSTT, DeepgramSTTConfig, STTConfig, STTConnectionState, STTError, STTProvider,
    STTResult, STTResultCallback, STTStats, create_stt_provider, create_stt_provider_from_enum,
    get_supported_stt_providers,
};

pub use tts::{
    AudioCallback, AudioData, BaseTTS, BoxedTTS, ConnectionState, DeepgramTTS, TTSConfig, TTSError,
    TTSFactory, TTSResult, create_tts_provider, get_tts_provider_urls,
};

pub use voice_manager::{
    STTCallback, TTSAudioCallback, TTSErrorCallback, VoiceManager, VoiceManagerConfig,
    VoiceManagerError, VoiceManagerResult,
};

// Re-export CoreState for external use
pub use state::CoreState;
