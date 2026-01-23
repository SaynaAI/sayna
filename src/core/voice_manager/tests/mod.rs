//! Tests for VoiceManager
//!
//! Per CLAUDE.md "Testing" section, tests are organized into modules:
//! - `basic`: Core creation, configuration, and callback registration tests
//! - `callbacks`: TTS callback atomic update tests
//! - `speech_final`: STT provider is_speech_final handling (non-VAD mode)
//! - `vad_failures`: VAD failure handling tests (VAD mode)
//!
//! Shared test utilities are in the `helpers` and `stubs` modules.

mod helpers;

mod basic;
mod callbacks;

#[cfg(not(feature = "stt-vad"))]
mod speech_final;

#[cfg(feature = "stt-vad")]
mod stubs;

#[cfg(feature = "stt-vad")]
mod vad_failures;
