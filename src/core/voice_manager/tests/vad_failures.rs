//! Tests for VAD failure not blocking STT audio processing.
//!
//! Per CLAUDE.md "Feature Flags" section, these tests are gated under `stt-vad`.
//! Per CLAUDE.md "Testing" section, async tests use `#[tokio::test]`.
//!
//! This module is feature-gated via `#[cfg(feature = "stt-vad")]` in the parent mod.rs.

use crate::core::stt::BaseSTT;
use crate::core::tts::{BaseTTS, TTSConfig};
use crate::core::vad::{SileroVAD, SileroVADConfig};
use crate::core::voice_manager::VoiceManager;
use crate::core::voice_manager::vad_processor::VADState;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::RwLock;
use tokio::time::Duration;

use super::helpers::stub_voice_manager_config;
use super::stubs::{AudioTracker, StubSTT, StubTTS};

/// Creates a connected StubSTT with the given audio tracker.
async fn create_connected_stub_stt(tracker: &AudioTracker) -> StubSTT {
    let mut stub_stt = StubSTT::new_with_tracking(
        tracker.send_audio_called.clone(),
        tracker.send_audio_call_count.clone(),
        tracker.audio_bytes_received.clone(),
    );
    stub_stt.connect().await.unwrap();
    stub_stt
}

/// Creates a connected StubTTS.
async fn create_connected_stub_tts() -> StubTTS {
    let mut stub_tts = StubTTS::new(TTSConfig::default()).unwrap();
    stub_tts.connect().await.unwrap();
    stub_tts
}

/// Creates a VoiceManager with stub providers and no VAD.
async fn create_vm_without_vad(tracker: &AudioTracker) -> VoiceManager {
    let stub_stt = create_connected_stub_stt(tracker).await;
    let stub_tts = create_connected_stub_tts().await;
    let config = stub_voice_manager_config();

    VoiceManager::new_with_providers(config, Box::new(stub_stt), Box::new(stub_tts), None, None)
        .unwrap()
}

/// Asserts that STT received the expected audio.
fn assert_stt_received_audio(tracker: &AudioTracker, expected_calls: usize, expected_bytes: usize) {
    assert!(
        tracker.send_audio_called.load(Ordering::SeqCst),
        "STT send_audio should have been called"
    );
    assert_eq!(
        tracker.send_audio_call_count.load(Ordering::SeqCst),
        expected_calls,
        "STT send_audio call count mismatch"
    );
    assert_eq!(
        tracker.audio_bytes_received.load(Ordering::SeqCst),
        expected_bytes,
        "STT audio bytes received mismatch"
    );
}

/// Test that `receive_audio` sends audio to STT when VAD is not available (None).
/// Regression test: STT should receive audio even when VAD is unavailable.
#[tokio::test]
async fn test_stt_receives_audio_when_vad_unavailable() {
    let tracker = AudioTracker::new();
    let vm = create_vm_without_vad(&tracker).await;

    let sample_audio = vec![0u8; 1024];
    let result = vm.receive_audio(sample_audio.clone()).await;

    assert!(
        result.is_ok(),
        "receive_audio should succeed when VAD is unavailable"
    );
    tokio::time::sleep(Duration::from_millis(10)).await;

    assert_stt_received_audio(&tracker, 1, sample_audio.len());
}

/// Test that multiple receive_audio calls all reach STT when VAD is unavailable.
#[tokio::test]
async fn test_multiple_audio_chunks_reach_stt_without_vad() {
    let tracker = AudioTracker::new();
    let vm = create_vm_without_vad(&tracker).await;

    let chunk_size = 1024;
    let num_chunks = 5;
    for _ in 0..num_chunks {
        let result = vm.receive_audio(vec![0u8; chunk_size]).await;
        assert!(result.is_ok());
    }

    tokio::time::sleep(Duration::from_millis(20)).await;

    assert_stt_received_audio(&tracker, num_chunks, chunk_size * num_chunks);
}

/// Test that STT receives audio even when VAD processing returns an error.
/// Regression test: VAD errors should not block STT audio processing.
#[tokio::test]
async fn test_stt_receives_audio_when_vad_processing_errors() {
    let vad = Arc::new(RwLock::new(SileroVAD::stub_for_testing(
        SileroVADConfig::default(),
    )));

    let tracker = AudioTracker::new();
    let stub_stt = create_connected_stub_stt(&tracker).await;
    let stub_tts = create_connected_stub_tts().await;
    let config = stub_voice_manager_config();

    let vad_state = Arc::new(VADState::new(16000 * 8, 512));
    vad_state.set_force_error(true);

    let vm = VoiceManager::new_with_providers_and_vad_state(
        config,
        Box::new(stub_stt),
        Box::new(stub_tts),
        None,
        Some(vad),
        vad_state,
    )
    .unwrap();

    let sample_audio = vec![0u8; 1024];
    let result = vm.receive_audio(sample_audio.clone()).await;

    assert!(
        result.is_ok(),
        "receive_audio should succeed even when VAD processing errors"
    );
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_stt_received_audio(&tracker, 1, sample_audio.len());
}
