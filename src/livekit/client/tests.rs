//! Unit tests for `LiveKitClient` public APIs and helpers.

use super::audio_pump::AudioPumpHandle;
use super::*;
use crate::AppError;
use livekit::webrtc::audio_source::native::NativeAudioSource;
use tokio::time::{Duration, timeout};
use tokio_util::sync::CancellationToken;

fn create_test_config() -> LiveKitConfig {
    LiveKitConfig {
        url: "wss://test-server.com".to_string(),
        token: "mock-jwt-token".to_string(),
        room_name: "test-room".to_string(),
        publish_audio: true,
        subscribe_audio: true,
        sample_rate: 24000,
        channels: 1,
        enable_noise_filter: cfg!(feature = "noise-filter"),
        listen_participants: vec![],
    }
}

fn create_data_only_test_config() -> LiveKitConfig {
    LiveKitConfig {
        publish_audio: false,
        subscribe_audio: false,
        enable_noise_filter: false,
        ..create_test_config()
    }
}

#[tokio::test]
async fn test_livekit_client_creation() {
    let config = create_test_config();
    let client = LiveKitClient::new(config);

    assert!(!client.is_connected());
    assert!(!client.has_audio_source());
    assert!(!client.has_local_audio_track());
    assert!(!client.has_local_track_publication().await);
}

#[tokio::test]
async fn test_livekit_client_creation_data_only() {
    let config = create_data_only_test_config();
    let client = LiveKitClient::new(config.clone());

    assert_eq!(
        config.media_mode(),
        crate::livekit::LiveKitMediaMode::DataOnly
    );
    assert!(!config.room_options().auto_subscribe);
    assert!(!client.is_connected());
    assert!(!client.has_audio_source());
    assert!(!client.has_local_audio_track());
    assert!(!client.has_local_track_publication().await);
}

#[test]
fn test_data_only_room_options_disable_auto_subscribe() {
    let config = create_data_only_test_config();

    assert!(!config.room_options().auto_subscribe);
    assert!(!config.publish_audio);
    assert!(!config.subscribe_audio);
    assert!(!config.enable_noise_filter);
}

#[tokio::test]
async fn test_livekit_client_clear_audio_not_connected() {
    let config = create_test_config();
    let client = LiveKitClient::new(config);

    let result = client.clear_audio().await;
    assert!(
        result.is_err(),
        "clear_audio should fail when not connected"
    );

    if let Err(AppError::InternalServerError(msg)) = result {
        assert!(msg.contains("Not connected"));
    } else {
        panic!("Expected InternalServerError about not being connected");
    }
}

#[tokio::test]
async fn test_livekit_client_clear_audio_no_audio_source() {
    let config = create_test_config();
    let client = LiveKitClient::new(config);

    client.set_connected(true).await;

    let result = client.clear_audio().await;
    assert!(
        result.is_ok(),
        "clear_audio should succeed when no audio source"
    );
}

#[tokio::test]
async fn test_livekit_client_callback_registration() {
    let config = create_test_config();
    let mut client = LiveKitClient::new(config);

    client.set_audio_callback(|_data| {});
    assert!(client.audio_callback.is_some());

    client.set_data_callback(|_data| {});
    assert!(client.data_callback.is_some());

    client.set_participant_disconnect_callback(|_event| {});
    assert!(client.participant_disconnect_callback.is_some());
}

#[test]
fn test_pcm_to_samples_decodes_little_endian() {
    // Bytes [0,1,2,3] little-endian -> i16 [0x0100, 0x0302] = [256, 770].
    let audio_data = vec![0u8, 1u8, 2u8, 3u8];
    let samples =
        LiveKitClient::pcm_to_samples(&audio_data, 1).expect("even byte count must decode");
    assert_eq!(samples, vec![256, 770]);
}

#[test]
fn test_pcm_to_samples_rejects_odd_length() {
    let audio_data = vec![0u8, 1u8, 2u8];
    let result = LiveKitClient::pcm_to_samples(&audio_data, 1);

    assert!(result.is_err());
    if let Err(AppError::InternalServerError(msg)) = result {
        assert!(msg.contains("even"));
    } else {
        panic!("expected an InternalServerError mentioning even length");
    }
}

#[test]
fn test_pcm_to_samples_rejects_partial_frame() {
    // One i16 sample cannot fill a stereo frame.
    let audio_data = vec![0u8, 1u8];
    assert!(LiveKitClient::pcm_to_samples(&audio_data, 2).is_err());
}

#[test]
fn test_audio_source_queue_size_ms_is_always_valid() {
    // libwebrtc asserts the source queue depth is a non-zero multiple of 10 ms.
    // Client-supplied TTS sample rates are arbitrary, so the depth must stay
    // valid for any of them — notably 44_100 Hz, which the old formula broke.
    use super::audio::audio_source_queue_size_ms;
    for rate in [
        8_000u32, 11_025, 16_000, 22_050, 24_000, 44_100, 48_000, 999, 1,
    ] {
        let ms = audio_source_queue_size_ms(rate);
        assert!(ms > 0, "queue depth must be non-zero at {rate} Hz");
        assert_eq!(
            ms % 10,
            0,
            "queue depth must be a multiple of 10 at {rate} Hz"
        );
    }
    // The nominal rates keep their historical ~sample_rate/100 ms depth.
    assert_eq!(audio_source_queue_size_ms(24_000), 240);
    assert_eq!(audio_source_queue_size_ms(48_000), 480);
}

#[tokio::test]
async fn test_livekit_client_clear_audio_timing() {
    let config = create_test_config();
    let client = LiveKitClient::new(config);
    client.set_connected(true).await;

    let result = timeout(Duration::from_millis(100), client.clear_audio()).await;

    assert!(result.is_ok(), "clear_audio should complete within 100ms");
    assert!(result.unwrap().is_ok());
}

#[tokio::test]
async fn test_livekit_client_send_tts_audio_not_connected() {
    let config = create_test_config();
    let client = LiveKitClient::new(config);

    let audio_data = vec![0u8; 1024];
    let result = client.send_tts_audio(audio_data).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_send_tts_audio_buffers_samples_when_connected() {
    let config = create_test_config();
    let client = LiveKitClient::new(config);
    client.set_connected(true).await;

    // 1024 bytes = 512 interleaved i16 samples (mono). With no pump running yet,
    // they accumulate in the shared TTS buffer for the pump to drain.
    let audio_data = vec![0u8; 1024];
    let result = client.send_tts_audio(audio_data).await;

    assert!(result.is_ok());
    assert_eq!(client.tts_buffer_len().await, 512);
}

#[tokio::test]
async fn test_livekit_client_multiple_clear_audio_calls() {
    let config = create_test_config();
    let client = LiveKitClient::new(config);
    client.set_connected(true).await;

    for _ in 0..3 {
        assert!(client.clear_audio().await.is_ok());
    }
}

#[tokio::test]
async fn test_livekit_client_clear_audio_state_consistency() {
    let config = create_test_config();
    let client = LiveKitClient::new(config);
    client.set_connected(true).await;

    assert!(client.is_connected());
    assert!(!client.has_audio_source());

    let result = client.clear_audio().await;
    assert!(result.is_ok());
    assert!(client.is_connected());
}

#[tokio::test]
async fn test_livekit_client_data_message_serialization() {
    use serde_json::json;

    let config = create_test_config();
    let client = LiveKitClient::new(config);
    client.set_connected(true).await;

    let test_data = json!({
        "type": "test",
        "message": "hello world"
    });

    // With the refactored code, send_data_message returns error when no room
    let result = client.send_data_message("test-topic", test_data).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_livekit_client_disconnect_cleanup() {
    let config = create_test_config();
    let mut client = LiveKitClient::new(config);
    client.set_connected(true).await;

    assert!(client.disconnect().await.is_ok());

    assert!(!client.is_connected());
    assert!(!client.has_audio_source());
    assert!(!client.has_local_audio_track());
    assert!(!client.has_local_track_publication().await);
    assert!(!client.has_room().await);
    assert!(!client.has_room_events());
}

#[tokio::test]
async fn test_livekit_client_clear_audio_integration_pattern() {
    let config = create_test_config();
    let client = LiveKitClient::new(config);
    client.set_connected(true).await;

    let result = client.clear_audio().await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_tts_buffer_accumulates_and_clear_empties_it() {
    let config = create_test_config();
    let client = LiveKitClient::new(config);
    client.set_connected(true).await;

    // Three 4-byte chunks = three 2-sample pushes = 6 buffered samples.
    let audio_data = vec![0u8, 1u8, 2u8, 3u8];
    for _ in 0..3 {
        let _ = client.send_tts_audio(audio_data.clone()).await;
    }
    assert_eq!(client.tts_buffer_len().await, 6);

    // Clear drains the buffer.
    let result = client.clear_audio().await;
    assert!(result.is_ok());
    assert_eq!(client.tts_buffer_len().await, 0);
}

#[tokio::test]
async fn test_livekit_client_message_send_with_serializable_struct() {
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    struct TestMessage {
        id: u32,
        text: String,
    }

    let config = create_test_config();
    let client = LiveKitClient::new(config);
    client.set_connected(true).await;

    let msg = TestMessage {
        id: 42,
        text: "test".to_string(),
    };

    // Should succeed with serialization but fail due to no room
    let result = client.send_data_message("test-topic", msg).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_buffered_tts_survives_connection_toggle() {
    let config = create_test_config();
    let client = LiveKitClient::new(config);
    client.set_connected(true).await;

    // 48 bytes = 24 interleaved i16 samples (mono).
    let audio_data = vec![0u8; 48];
    let _ = client.send_tts_audio(audio_data).await;
    assert_eq!(client.tts_buffer_len().await, 24);

    // Buffered audio is retained across connection-status changes; only the new
    // pump (on reconnect) or an explicit clear consumes it.
    client.set_connected(false).await;
    assert_eq!(client.tts_buffer_len().await, 24);

    client.set_connected(true).await;
    assert_eq!(client.tts_buffer_len().await, 24);
}

#[tokio::test]
async fn test_livekit_client_operation_priority_ordering() {
    let config = create_test_config();
    let client = LiveKitClient::new(config);

    // Get operation queue and verify it's available
    let queue = client.get_operation_queue();
    assert!(queue.is_none()); // Queue only exists after connect
}

#[tokio::test]
async fn test_loading_overlay_default_state() {
    let config = create_test_config();
    let client = LiveKitClient::new(config);

    assert!(!client.has_loading_samples());
    assert!(!client.loading_overlay_active());
    assert!(!client.pump_running().await);
}

#[tokio::test]
async fn test_set_loading_audio_clip_resamples_and_stores() {
    // The canonical 16 kHz mono clip is resampled to the 24 kHz mono track.
    let clip = crate::livekit::loading_clip::make_test_loading_clip();

    let config = create_test_config();
    let mut client = LiveKitClient::new(config);
    assert!(!client.has_loading_samples());

    client
        .set_loading_audio_clip(clip)
        .expect("resampling the clip to the track format should succeed");
    assert!(client.has_loading_samples());
}

#[tokio::test]
async fn test_stop_loading_audio_with_no_overlay_is_noop() {
    let config = create_test_config();
    let client = LiveKitClient::new(config);

    // Stopping with no overlay active must not panic and must leave it inactive.
    client.stop_loading_audio().await;
    assert!(!client.loading_overlay_active());
}

#[tokio::test]
async fn test_start_loading_audio_not_connected_errors() {
    let config = create_test_config();
    let client = LiveKitClient::new(config);

    let err = client
        .start_loading_audio()
        .await
        .expect_err("start_loading_audio should fail when not connected");
    match err {
        AppError::BadRequest(msg) => assert!(
            msg.contains("active LiveKit connection"),
            "unexpected error message: {msg}"
        ),
        other => panic!("expected BadRequest when not connected, got {other:?}"),
    }
}

#[tokio::test]
async fn test_start_loading_audio_connected_without_clip_errors() {
    let config = create_test_config();
    let client = LiveKitClient::new(config);
    client.set_connected(true).await;

    // Connected, but no loading clip was ever configured. The error must name
    // that specific cause rather than a generic message.
    let err = client
        .start_loading_audio()
        .await
        .expect_err("start_loading_audio should fail without a loading clip");
    match err {
        AppError::BadRequest(msg) => assert!(
            msg.contains("no loading audio configured"),
            "unexpected error message: {msg}"
        ),
        other => panic!("expected BadRequest for the missing-clip case, got {other:?}"),
    }
}

#[tokio::test]
async fn test_start_loading_audio_with_clip_activates_overlay() {
    // Connected with a configured clip: the overlay flag flips on. (No pump is
    // spawned here, so nothing plays — the flag is what the pump consumes.)
    let clip = crate::livekit::loading_clip::make_test_loading_clip();

    let mut client = LiveKitClient::new(create_test_config());
    client.set_connected(true).await;
    client
        .set_loading_audio_clip(clip)
        .expect("resampling should succeed");

    assert!(!client.loading_overlay_active());
    client
        .start_loading_audio()
        .await
        .expect("start_loading_audio should succeed with a clip while connected");
    assert!(client.loading_overlay_active());

    // A second start while active is an idempotent no-op.
    client
        .start_loading_audio()
        .await
        .expect("second start_loading_audio should be an idempotent no-op");
    assert!(client.loading_overlay_active());
}

#[tokio::test]
async fn test_pump_running_false_when_handle_finished() {
    let config = create_test_config();
    let client = LiveKitClient::new(config);

    let handle = tokio::spawn(async {});
    for _ in 0..32 {
        if handle.is_finished() {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        handle.is_finished(),
        "placeholder task should finish promptly"
    );

    *client.pump.lock().await = Some(AudioPumpHandle {
        handle,
        token: CancellationToken::new(),
    });

    assert!(
        !client.pump_running().await,
        "a finished JoinHandle must not count as a running pump"
    );
}

#[tokio::test]
async fn test_disconnect_clears_loading_overlay_state() {
    let clip = crate::livekit::loading_clip::make_test_loading_clip();

    let mut client = LiveKitClient::new(create_test_config());
    client.set_connected(true).await;
    client
        .set_loading_audio_clip(clip)
        .expect("resampling should succeed");
    client
        .start_loading_audio()
        .await
        .expect("start_loading_audio should succeed");
    assert!(client.has_loading_samples());
    assert!(client.loading_overlay_active());

    assert!(client.disconnect().await.is_ok());

    assert!(!client.has_loading_samples());
    assert!(!client.loading_overlay_active());
    assert!(!client.pump_running().await);
}

// NOTE: the `livekit_native_*` tests below construct real libwebrtc
// `NativeAudioSource` objects. libwebrtc's lazily-initialised global runtime is
// not robust against the heavy thread concurrency of the full unit-test binary
// and intermittently segfaults when these run alongside the rest of the suite.
// They are therefore `#[ignore]`d out of the default `cargo test` run and
// executed isolated (their own process) by a dedicated CI step — see
// `.github/workflows/ci.yml`. Run locally with:
//   cargo test --all-features --lib -- --ignored --test-threads=1 livekit_native_

/// Builds a connected `LiveKitClient` with a loading clip configured and a real
/// audio pump spawned on a native source, ready to exercise overlay start/stop.
///
/// The pump and source are wired directly because the normal setup path
/// (`setup_audio_publishing`) needs a live room, which unit tests do not have.
async fn connected_client_with_pump() -> LiveKitClient {
    let clip = crate::livekit::loading_clip::make_test_loading_clip();

    let mut client = LiveKitClient::new(create_test_config());
    client.set_connected(true).await;
    client
        .set_loading_audio_clip(clip)
        .expect("resampling should succeed");

    // 24 kHz mono source matching the test config; 240 ms queue mirrors setup.
    let source = Arc::new(NativeAudioSource::new(
        super::sayna_audio_source_options(),
        24_000,
        1,
        240,
    ));
    *client.audio_source.lock().await = Some(Arc::clone(&source));
    super::audio_pump::spawn_audio_pump(
        &client.pump,
        Arc::clone(&client.audio_pump),
        source,
        24_000,
        1,
    )
    .await;

    client
}

#[tokio::test]
#[ignore = "creates native libwebrtc objects; run isolated via the dedicated CI step (see ci.yml)"]
async fn livekit_native_start_idempotent_and_disconnect_teardown() {
    let mut client = connected_client_with_pump().await;

    client
        .start_loading_audio()
        .await
        .expect("first start_loading_audio should succeed");
    assert!(client.loading_overlay_active());
    assert!(client.pump_running().await);

    // A second start while the overlay is active is an idempotent no-op.
    client
        .start_loading_audio()
        .await
        .expect("second start_loading_audio should be an idempotent no-op");
    assert!(client.loading_overlay_active());

    // Disconnect stops the pump and clears the overlay state.
    client
        .disconnect()
        .await
        .expect("disconnect should succeed");
    assert!(!client.pump_running().await);
    assert!(!client.loading_overlay_active());
    assert!(!client.has_loading_samples());
}

#[tokio::test]
#[ignore = "creates native libwebrtc objects; run isolated via the dedicated CI step (see ci.yml)"]
async fn livekit_native_overlay_deactivates_after_stop() {
    let client = connected_client_with_pump().await;

    client
        .start_loading_audio()
        .await
        .expect("start_loading_audio should succeed");
    assert!(client.loading_overlay_active());

    // Requesting a stop triggers the fade-out; once it completes the pump flips
    // the overlay inactive and returns to pass-through.
    client.stop_loading_audio().await;

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline {
        if !client.loading_overlay_active() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("overlay must become inactive after stop and fade-out complete");
}

#[tokio::test]
#[ignore = "creates native libwebrtc objects; run isolated via the dedicated CI step (see ci.yml)"]
async fn livekit_native_rapid_start_stop_is_clean() {
    let mut client = connected_client_with_pump().await;

    // Rapidly alternating start/stop must never panic, and a disconnect
    // afterwards (with the overlay possibly mid-fade) must still tear down
    // cleanly without hanging.
    for _ in 0..30 {
        client
            .start_loading_audio()
            .await
            .expect("start_loading_audio should not error");
        client.stop_loading_audio().await;
    }

    let teardown = timeout(Duration::from_secs(5), client.disconnect()).await;
    assert!(
        teardown.is_ok(),
        "disconnect must not hang after rapid start/stop"
    );
    assert!(teardown.expect("disconnect completed").is_ok());
    assert!(!client.pump_running().await);
}

#[tokio::test]
#[ignore = "creates native libwebrtc objects; run isolated via the dedicated CI step (see ci.yml)"]
async fn livekit_native_drop_cancels_pump() {
    let client = connected_client_with_pump().await;
    client
        .start_loading_audio()
        .await
        .expect("start_loading_audio should succeed");

    // Hold a clone of the running pump's cancellation token, then drop the
    // client WITHOUT calling disconnect — the path session cleanup takes when it
    // cannot acquire the LiveKit lock in time.
    let token = client
        .pump_token_for_test()
        .await
        .expect("a pump should be running");
    assert!(!token.is_cancelled());

    drop(client);

    // The Drop backstop must have cancelled the pump so it cannot be orphaned.
    assert!(
        token.is_cancelled(),
        "dropping the client must cancel the audio pump"
    );
}
