//! Stub implementations for testing VoiceManager.
//!
//! Per CLAUDE.md "Testing" section, use mocking libraries or stub implementations
//! to test components in isolation.
//!
//! This module is feature-gated via `#[cfg(feature = "stt-vad")]` in the parent mod.rs.

use crate::core::stt::{BaseSTT, STTConfig, STTError, STTErrorCallback, STTResultCallback};
use crate::core::tts::{AudioCallback, BaseTTS, ConnectionState, TTSConfig, TTSError};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Stub STT implementation that records `send_audio` calls.
///
/// This test-only provider tracks:
/// - `send_audio_called`: Whether `send_audio` was invoked
/// - `send_audio_call_count`: Total number of `send_audio` invocations
/// - `audio_bytes_received`: Total bytes received across all calls
pub struct StubSTT {
    config: Option<STTConfig>,
    connected: AtomicBool,
    send_audio_called: Arc<AtomicBool>,
    send_audio_call_count: Arc<AtomicUsize>,
    audio_bytes_received: Arc<AtomicUsize>,
}

impl StubSTT {
    pub fn new_with_tracking(
        send_audio_called: Arc<AtomicBool>,
        send_audio_call_count: Arc<AtomicUsize>,
        audio_bytes_received: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            config: Some(STTConfig::default()),
            connected: AtomicBool::new(false),
            send_audio_called,
            send_audio_call_count,
            audio_bytes_received,
        }
    }
}

#[async_trait::async_trait]
impl BaseSTT for StubSTT {
    fn new(config: STTConfig) -> Result<Self, STTError>
    where
        Self: Sized,
    {
        Ok(Self {
            config: Some(config),
            connected: AtomicBool::new(false),
            send_audio_called: Arc::new(AtomicBool::new(false)),
            send_audio_call_count: Arc::new(AtomicUsize::new(0)),
            audio_bytes_received: Arc::new(AtomicUsize::new(0)),
        })
    }

    async fn connect(&mut self) -> Result<(), STTError> {
        self.connected.store(true, Ordering::Relaxed);
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), STTError> {
        self.connected.store(false, Ordering::Relaxed);
        Ok(())
    }

    fn is_ready(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    async fn send_audio(&mut self, audio_data: Vec<u8>) -> Result<(), STTError> {
        self.send_audio_called.store(true, Ordering::SeqCst);
        self.send_audio_call_count.fetch_add(1, Ordering::SeqCst);
        self.audio_bytes_received
            .fetch_add(audio_data.len(), Ordering::SeqCst);
        Ok(())
    }

    async fn on_result(&mut self, _callback: STTResultCallback) -> Result<(), STTError> {
        Ok(())
    }

    async fn on_error(&mut self, _callback: STTErrorCallback) -> Result<(), STTError> {
        Ok(())
    }

    fn get_config(&self) -> Option<&STTConfig> {
        self.config.as_ref()
    }

    async fn update_config(&mut self, config: STTConfig) -> Result<(), STTError> {
        self.config = Some(config);
        Ok(())
    }

    fn get_provider_info(&self) -> &'static str {
        "StubSTT v1.0 (test-only)"
    }
}

/// Stub TTS implementation for testing VoiceManager.
pub struct StubTTS {
    connected: bool,
}

#[async_trait::async_trait]
impl BaseTTS for StubTTS {
    fn new(_config: TTSConfig) -> Result<Self, TTSError>
    where
        Self: Sized,
    {
        Ok(Self { connected: false })
    }

    async fn connect(&mut self) -> Result<(), TTSError> {
        self.connected = true;
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), TTSError> {
        self.connected = false;
        Ok(())
    }

    fn is_ready(&self) -> bool {
        self.connected
    }

    fn get_connection_state(&self) -> ConnectionState {
        if self.connected {
            ConnectionState::Connected
        } else {
            ConnectionState::Disconnected
        }
    }

    async fn speak(&mut self, _text: &str, _flush: bool) -> Result<(), TTSError> {
        Ok(())
    }

    async fn clear(&mut self) -> Result<(), TTSError> {
        Ok(())
    }

    async fn flush(&self) -> Result<(), TTSError> {
        Ok(())
    }

    fn on_audio(&mut self, _callback: Arc<dyn AudioCallback>) -> Result<(), TTSError> {
        Ok(())
    }

    fn remove_audio_callback(&mut self) -> Result<(), TTSError> {
        Ok(())
    }

    fn get_provider_info(&self) -> serde_json::Value {
        serde_json::json!({
            "provider": "stub",
            "version": "1.0.0"
        })
    }
}

/// Tracking state for stub STT audio calls.
pub struct AudioTracker {
    pub send_audio_called: Arc<AtomicBool>,
    pub send_audio_call_count: Arc<AtomicUsize>,
    pub audio_bytes_received: Arc<AtomicUsize>,
}

impl AudioTracker {
    pub fn new() -> Self {
        Self {
            send_audio_called: Arc::new(AtomicBool::new(false)),
            send_audio_call_count: Arc::new(AtomicUsize::new(0)),
            audio_bytes_received: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl Default for AudioTracker {
    fn default() -> Self {
        Self::new()
    }
}
