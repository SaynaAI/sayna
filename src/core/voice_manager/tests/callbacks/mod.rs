//! Tests for VoiceManager TTS callbacks.
//!
//! Per CLAUDE.md "Testing" section, tests are organized into focused modules:
//! - `interruption_timing`: Atomic update tests for non_interruptible_until_ms
//! - `duration_metadata`: Duration metadata preference and PCM calculation tests
//! - `pcm_formats`: Table-driven tests for PCM format variant recognition
//!
//! Shared test utilities are in the `helpers` module.

mod helpers;

mod duration_metadata;
mod interruption_timing;
mod pcm_formats;
