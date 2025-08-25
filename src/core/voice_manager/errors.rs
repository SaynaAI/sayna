//! Error types for VoiceManager operations

use crate::core::{stt::STTError, tts::TTSError};

/// Error types for VoiceManager operations
#[derive(Debug, thiserror::Error)]
pub enum VoiceManagerError {
    #[error("TTS error: {0}")]
    TTSError(#[from] TTSError),
    #[error("STT error: {0}")]
    STTError(#[from] STTError),
    #[error("Initialization error: {0}")]
    InitializationError(String),
    #[error("Provider not ready: {0}")]
    ProviderNotReady(String),
    #[error("Callback registration error: {0}")]
    CallbackRegistrationError(String),
    #[error("Internal error: {0}")]
    InternalError(String),
}

/// Result type for VoiceManager operations
pub type VoiceManagerResult<T> = Result<T, VoiceManagerError>;
