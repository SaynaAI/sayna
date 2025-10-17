//! Unit tests for `LiveKitClient` public APIs and helpers.

use super::*;
use crate::AppError;
use tokio::time::{Duration, timeout};

fn create_test_config() -> LiveKitConfig {
    LiveKitConfig {
        url: "wss://test-server.com".to_string(),
        token: "mock-jwt-token".to_string(),
        room_name: "test-room".to_string(),
        sample_rate: 24000,
        channels: 1,
        enable_noise_filter: cfg!(feature = "noise-filter"),
        listen_participants: vec![],
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
