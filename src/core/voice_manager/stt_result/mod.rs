//! STT result processing with timing control.
//!
//! Handles STT results with VAD-based silence detection. When `stt-vad` is compiled,
//! VAD mode is always active. All timeout-based fallbacks have been removed.
//!
//! Speech final events are emitted based on feature flag:
//! - When `stt-vad` enabled: VAD + Smart-Turn ONLY (STT provider's is_speech_final is IGNORED)
//! - When `stt-vad` NOT enabled: STT provider's `is_speech_final` ONLY
//!
//! ## Module Organization
//!
//! This module is split into focused submodules following the CLAUDE.md guidance
//! for VoiceManager (see "High-Level Architecture" → "VoiceManager"):
//!
//! - `processor`: Core STT result processing (result filtering, duplicate detection,
//!   turn detection state management)
//! - `vad_events`: VAD event handling (feature-gated: `stt-vad`)
//!
//! See CLAUDE.md for detailed architecture documentation.

mod processor;
#[cfg(feature = "stt-vad")]
mod vad_events;

pub use processor::STTResultProcessor;
