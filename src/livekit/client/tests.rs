//! Unit tests for `LiveKitClient` public APIs and helpers.

use super::*;
use crate::AppError;
use tokio::time::{Duration, timeout};

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

#[tokio::test]
async fn test_livekit_client_audio_frame_conversion() {
    let config = create_test_config();
    let audio_data = vec![0u8, 1u8, 2u8, 3u8];
    let result =
        LiveKitClient::convert_audio_to_frame_ref(&audio_data, config.sample_rate, config.channels);

    assert!(result.is_ok());
    let audio_frame = result.unwrap();
    assert_eq!(audio_frame.sample_rate, 24000);
    assert_eq!(audio_frame.num_channels, 1);
    assert_eq!(audio_frame.samples_per_channel, 2);
}

#[tokio::test]
async fn test_livekit_client_audio_frame_conversion_invalid_data() {
    let config = create_test_config();
    let audio_data = vec![0u8, 1u8, 2u8];
    let result =
        LiveKitClient::convert_audio_to_frame_ref(&audio_data, config.sample_rate, config.channels);

    assert!(result.is_err());
    if let Err(AppError::InternalServerError(msg)) = result {
        assert!(msg.contains("even"));
    }
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
async fn test_livekit_client_send_tts_audio_no_source() {
    let config = create_test_config();
    let client = LiveKitClient::new(config);
    client.set_connected(true).await;

    let audio_data = vec![0u8; 1024];
    let result = client.send_tts_audio(audio_data.clone()).await;

    assert!(result.is_ok());
    let queue_len = client.get_audio_queue_len().await;
    assert_eq!(queue_len, 1);
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
async fn test_livekit_client_audio_queue_drain_success() {
    let config = create_test_config();
    let client = LiveKitClient::new(config);
    client.set_connected(true).await;

    // Queue multiple audio frames
    let audio_data = vec![0u8, 1u8, 2u8, 3u8];
    for _ in 0..3 {
        let _ = client.send_tts_audio(audio_data.clone()).await;
    }

    // Verify frames were queued
    let queue_len = client.get_audio_queue_len().await;
    assert_eq!(queue_len, 3);

    // Clear the queue
    let result = client.clear_audio().await;
    assert!(result.is_ok());
    assert_eq!(client.get_audio_queue_len().await, 0);
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
async fn test_livekit_client_reconnect_audio_queue_preservation() {
    let config = create_test_config();
    let client = LiveKitClient::new(config);
    client.set_connected(true).await;

    // Queue audio data
    let audio_data = vec![0u8; 48];
    let _ = client.send_tts_audio(audio_data.clone()).await;
    assert_eq!(client.get_audio_queue_len().await, 1);

    // Audio should remain queued after status changes
    client.set_connected(false).await;
    assert_eq!(client.get_audio_queue_len().await, 1);

    client.set_connected(true).await;
    assert_eq!(client.get_audio_queue_len().await, 1);
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
async fn test_livekit_client_loading_fields_default() {
    let config = create_test_config();
    let client = LiveKitClient::new(config);

    assert!(!client.has_loading_clip());
    assert!(!client.has_loading_audio_source().await);
    assert!(!client.has_loading_audio_track());
    assert!(!client.has_loading_track_publication().await);
    assert!(!client.has_loading_loop().await);
}

#[tokio::test]
async fn test_set_loading_audio_clip_stores_clip() {
    let clip = crate::livekit::loading_clip::make_test_loading_clip();

    let config = create_test_config();
    let mut client = LiveKitClient::new(config);
    assert!(!client.has_loading_clip());

    client.set_loading_audio_clip(clip);
    assert!(client.has_loading_clip());
}

#[tokio::test]
async fn test_setup_loading_audio_track_without_clip_is_noop() {
    let mut client = LiveKitClient::new(create_test_config());

    // With no loading clip configured, track setup is a clean no-op — it
    // returns Ok and publishes nothing, leaving every loading field empty.
    client
        .setup_loading_audio_track()
        .await
        .expect("setup_loading_audio_track must be a no-op when no clip is configured");

    assert!(!client.has_loading_audio_source().await);
    assert!(!client.has_loading_audio_track());
    assert!(!client.has_loading_track_publication().await);
}

#[tokio::test]
async fn test_setup_loading_audio_track_with_clip_requires_room() {
    let clip = crate::livekit::loading_clip::make_test_loading_clip();

    let mut client = LiveKitClient::new(create_test_config());
    client.set_loading_audio_clip(clip);

    // A clip is configured but no room is connected: setup must surface an
    // error rather than silently succeed, confirming the clip-present path is
    // taken and the track is not published without a room.
    let result = client.setup_loading_audio_track().await;
    assert!(
        result.is_err(),
        "setup_loading_audio_track must error when a clip is set but no room is available"
    );
    assert!(!client.has_loading_audio_source().await);
    assert!(!client.has_loading_track_publication().await);
}

#[tokio::test]
async fn test_stop_loading_audio_no_loop_is_noop() {
    let config = create_test_config();
    let client = LiveKitClient::new(config);

    // Stopping with no loop running must not panic and must leave no loop.
    client.stop_loading_audio().await;
    assert!(!client.has_loading_loop().await);
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
async fn test_has_loading_loop_false_when_handle_finished() {
    use super::loading_audio::LoadingLoopHandle;

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

    *client.loading_loop.lock().await = Some(LoadingLoopHandle {
        handle,
        token: tokio_util::sync::CancellationToken::new(),
    });

    assert!(
        !client.has_loading_loop().await,
        "a finished JoinHandle must not count as an active loading loop"
    );
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
async fn test_start_loading_audio_with_clip_but_no_source_errors() {
    // A clip is configured but the dedicated loading track never published, so
    // there is no audio source. start_loading_audio must report that specific
    // cause, distinct from the missing-clip message.
    let clip = crate::livekit::loading_clip::make_test_loading_clip();

    let mut client = LiveKitClient::new(create_test_config());
    client.set_connected(true).await;
    client.set_loading_audio_clip(clip);

    let err = client
        .start_loading_audio()
        .await
        .expect_err("start_loading_audio should fail with a clip but no published source");
    match err {
        AppError::BadRequest(msg) => assert!(
            msg.contains("failed to publish"),
            "unexpected error message: {msg}"
        ),
        other => panic!("expected BadRequest for the missing-source case, got {other:?}"),
    }
}

#[tokio::test]
async fn test_disconnect_clears_loading_fields() {
    let config = create_test_config();
    let mut client = LiveKitClient::new(config);
    client.set_connected(true).await;

    assert!(client.disconnect().await.is_ok());

    assert!(!client.has_loading_clip());
    assert!(!client.has_loading_audio_source().await);
    assert!(!client.has_loading_audio_track());
    assert!(!client.has_loading_track_publication().await);
    assert!(!client.has_loading_loop().await);
}

// NOTE: the `livekit_native_*` tests below construct real libwebrtc
// `NativeAudioSource` objects. libwebrtc's lazily-initialised global runtime is
// not robust against the heavy thread concurrency of the full unit-test binary
// and intermittently segfaults when these run alongside the rest of the suite.
// They are therefore `#[ignore]`d out of the default `cargo test` run and
// executed isolated (their own process) by a dedicated CI step — see
// `.github/workflows/ci.yml`. Run locally with:
//   cargo test --all-features --lib -- --ignored --test-threads=1 livekit_native_
#[tokio::test]
#[ignore = "creates native libwebrtc objects; run isolated via the dedicated CI step (see ci.yml)"]
async fn livekit_native_two_independent_audio_sources() {
    use livekit::track::LocalAudioTrack;
    use livekit::webrtc::audio_source::native::NativeAudioSource;
    use livekit::webrtc::prelude::RtcAudioSource;

    // TTS-style source at 24 kHz mono.
    let tts_source = NativeAudioSource::new(super::sayna_audio_source_options(), 24_000, 1, 240);
    let tts_track =
        LocalAudioTrack::create_audio_track("tts-audio", RtcAudioSource::Native(tts_source));

    // Loading-style source at a different sample rate, 16 kHz mono.
    let loading_source =
        NativeAudioSource::new(super::sayna_audio_source_options(), 16_000, 1, 160);
    let loading_track = LocalAudioTrack::create_audio_track(
        "loading-audio",
        RtcAudioSource::Native(loading_source),
    );

    // Both tracks must construct without panic, confirming a participant can
    // hold two independent audio sources/tracks at different sample rates.
    assert_eq!(tts_track.name(), "tts-audio");
    assert_eq!(loading_track.name(), "loading-audio");
}

/// Builds a connected `LiveKitClient` with a loading clip set and a loading
/// audio source injected, ready to exercise the loading loop. The source is
/// injected directly because `setup_loading_audio_track` needs a live room,
/// which unit tests do not have.
async fn connected_client_with_loading_source() -> LiveKitClient {
    use livekit::webrtc::audio_source::native::NativeAudioSource;

    let clip = crate::livekit::loading_clip::make_test_loading_clip();

    let mut client = LiveKitClient::new(create_test_config());
    client.set_connected(true).await;
    client.set_loading_audio_clip(clip);

    let source = Arc::new(NativeAudioSource::new(
        super::sayna_audio_source_options(),
        16_000,
        1,
        super::loading_audio::LOADING_AUDIO_QUEUE_SIZE_MS,
    ));
    *client.loading_audio_source.lock().await = Some(source);
    client
}

#[tokio::test]
#[ignore = "creates native libwebrtc objects; run isolated via the dedicated CI step (see ci.yml)"]
async fn livekit_native_start_idempotent_and_disconnect_teardown() {
    let mut client = connected_client_with_loading_source().await;

    // The first start spawns the loop.
    client
        .start_loading_audio()
        .await
        .expect("first start_loading_audio should succeed");
    assert!(client.has_loading_loop().await);

    // A second start while the loop is still running is an idempotent no-op.
    client
        .start_loading_audio()
        .await
        .expect("second start_loading_audio should be an idempotent no-op");
    assert!(client.has_loading_loop().await);

    // Disconnect cancels the loop, awaits it (with the abort backstop), and
    // clears every loading field.
    client
        .disconnect()
        .await
        .expect("disconnect should succeed");
    assert!(!client.has_loading_loop().await);
    assert!(!client.has_loading_audio_source().await);
    assert!(!client.has_loading_clip());
}

#[tokio::test]
#[ignore = "creates native libwebrtc objects; run isolated via the dedicated CI step (see ci.yml)"]
async fn livekit_native_loading_loop_cleared_after_stop() {
    let client = connected_client_with_loading_source().await;

    client
        .start_loading_audio()
        .await
        .expect("start_loading_audio should succeed");
    assert!(client.has_loading_loop().await);

    client.stop_loading_audio().await;

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline {
        if !client.has_loading_loop().await {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("loading_loop slot must be cleared after stop and fade-out complete");
}

#[tokio::test]
#[ignore = "creates native libwebrtc objects; run isolated via the dedicated CI step (see ci.yml)"]
async fn livekit_native_rapid_start_stop_is_clean() {
    let mut client = connected_client_with_loading_source().await;

    // Rapidly alternating start/stop must never panic, and a disconnect
    // afterwards (with the loop possibly mid-fade) must still tear down cleanly
    // without hanging.
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
    assert!(!client.has_loading_loop().await);
    assert!(!client.has_loading_audio_source().await);
}

#[tokio::test]
#[ignore = "creates native libwebrtc objects; run isolated via the dedicated CI step (see ci.yml)"]
async fn livekit_native_drop_cancels_active_loop() {
    let client = connected_client_with_loading_source().await;
    client
        .start_loading_audio()
        .await
        .expect("start_loading_audio should succeed");

    // Hold a clone of the running loop's cancellation token, then drop the
    // client WITHOUT calling disconnect — the path session cleanup takes when
    // it cannot acquire the LiveKit lock in time.
    let token = client
        .loading_loop_token_for_test()
        .await
        .expect("a loop should be running");
    assert!(!token.is_cancelled());

    drop(client);

    // The Drop backstop must have cancelled the loop so it cannot be orphaned.
    assert!(
        token.is_cancelled(),
        "dropping the client must cancel the loading loop"
    );
}
