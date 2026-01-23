//! Speech final timing control tests for VoiceManager.
//!
//! These tests verify STT provider's is_speech_final handling, which is only
//! active when stt-vad is NOT enabled. When stt-vad is enabled, is_speech_final
//! from STT providers is ignored and VAD + Smart Turn controls turn completion.
//!
//! Per CLAUDE.md "Testing" section, unit tests use `#[tokio::test]` for async tests.
//! Per CLAUDE.md "Feature Flags" section, these tests are gated under `not(feature = "stt-vad")`.

#![cfg(not(feature = "stt-vad"))]

mod duplicates;
mod helpers;
mod timing;
