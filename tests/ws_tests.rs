use futures::{SinkExt, StreamExt};
use serde_json::json;
use std::io::ErrorKind;

use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

use sayna::{ServerConfig, routes, state::AppState};

#[tokio::test]
async fn test_websocket_voice_config() {
    // Create test config
    let config = ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 0, // Let the OS assign a port
        livekit_url: "ws://localhost:7880".to_string(),
        deepgram_api_key: Some("test_key".to_string()),
        elevenlabs_api_key: Some("test_key".to_string()),
        cache_path: None,
        cache_ttl_seconds: Some(3600),
    };

    // Create application state
    let app_state = AppState::new(config.clone()).await;

    // Create router
    let app = routes::api::create_api_router()
        .merge(routes::ws::create_ws_router())
        .with_state(app_state);

    // Create listener
    let listener = match TcpListener::bind("127.0.0.1:0").await {
        Ok(listener) => listener,
        Err(err) => {
            if err.kind() == ErrorKind::PermissionDenied {
                eprintln!("Skipping test_websocket_voice_config: {err}");
                return;
            }
            panic!("Failed to bind WebSocket test listener: {err}");
        }
    };
    let addr = listener.local_addr().unwrap();

    // Start server in background
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Give server time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Connect to WebSocket
    let url = format!("ws://127.0.0.1:{}/ws", addr.port());
    let (ws_stream, _) = connect_async(url).await.expect("Failed to connect");
    let (mut write, mut read) = ws_stream.split();

    // Send config message
    let config_message = json!({
        "type": "config",
        "stt_config": {
            "provider": "deepgram",
            "api_key": "test_key",
            "language": "en-US",
            "sample_rate": 16000,
            "channels": 1,
            "punctuation": true
        },
        "tts_config": {
            "provider": "deepgram",
            "api_key": "test_key",
            "voice_id": "aura-luna-en",
            "speaking_rate": 1.0,
            "audio_format": "pcm",
            "sample_rate": 22050,
            "connection_timeout": 30,
            "request_timeout": 60
        }
    });

    write
        .send(Message::Text(config_message.to_string().into()))
        .await
        .unwrap();

    // Receive response (should be ready or error)
    let response = read.next().await.unwrap().unwrap();

    // Check response
    match response {
        Message::Text(text) => {
            let text_str = text.to_string();
            let parsed: serde_json::Value = serde_json::from_str(&text_str).unwrap();
            let msg_type = parsed["type"].as_str().unwrap();

            // Accept either "ready" or "error" since we're using test keys
            assert!(
                msg_type == "ready" || msg_type == "error",
                "Expected 'ready' or 'error' message, got: {msg_type}"
            );

            if msg_type == "error" {
                println!(
                    "Expected error due to test API keys: {}",
                    parsed["message"].as_str().unwrap()
                );
            }
        }
        _ => panic!("Expected text message"),
    }

    // Test speak message (should work even with test keys for basic validation)
    let speak_message = json!({
        "type": "speak",
        "text": "Hello, world!",
        "flush": true
    });

    write
        .send(Message::Text(speak_message.to_string().into()))
        .await
        .unwrap();

    // Test clear message
    let clear_message = json!({
        "type": "clear"
    });

    write
        .send(Message::Text(clear_message.to_string().into()))
        .await
        .unwrap();

    // Test audio message
    let audio_message = json!({
        "type": "audio",
        "data": "dGVzdCBhdWRpbyBkYXRh" // base64 encoded "test audio data"
    });

    write
        .send(Message::Text(audio_message.to_string().into()))
        .await
        .unwrap();

    // Test binary audio message
    let test_binary = vec![1, 2, 3, 4, 5];
    write
        .send(Message::Binary(test_binary.into()))
        .await
        .unwrap();

    // Close connection
    write.close().await.unwrap();
}

#[tokio::test]
async fn test_websocket_invalid_message() {
    // Create test config
    let config = ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        livekit_url: "ws://localhost:7880".to_string(),
        deepgram_api_key: Some("test_key".to_string()),
        elevenlabs_api_key: Some("test_key".to_string()),
        cache_path: None,
        cache_ttl_seconds: Some(3600),
    };

    // Create application state
    let app_state = AppState::new(config.clone()).await;

    // Create router
    let app = routes::api::create_api_router()
        .merge(routes::ws::create_ws_router())
        .with_state(app_state);

    // Create listener
    let listener = match TcpListener::bind("127.0.0.1:0").await {
        Ok(listener) => listener,
        Err(err) => {
            if err.kind() == ErrorKind::PermissionDenied {
                eprintln!("Skipping test_websocket_invalid_message: {err}");
                return;
            }
            panic!("Failed to bind WebSocket test listener: {err}");
        }
    };
    let addr = listener.local_addr().unwrap();

    // Start server in background
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Give server time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Connect to WebSocket
    let url = format!("ws://127.0.0.1:{}/ws", addr.port());
    let (ws_stream, _) = connect_async(url).await.expect("Failed to connect");
    let (mut write, mut read) = ws_stream.split();

    // Send invalid message (plain text instead of JSON)
    let invalid_message = "Hello, WebSocket!";
    write
        .send(Message::Text(invalid_message.into()))
        .await
        .unwrap();

    // Receive error response
    let response = read.next().await.unwrap().unwrap();

    // Check that we get an error message
    match response {
        Message::Text(text) => {
            let text_str = text.to_string();
            let parsed: serde_json::Value = serde_json::from_str(&text_str).unwrap();
            assert_eq!(parsed["type"].as_str(), Some("error"));
            assert!(
                parsed["message"]
                    .as_str()
                    .unwrap()
                    .contains("Invalid message format")
            );
        }
        _ => panic!("Expected text message"),
    }

    // Close connection
    write.close().await.unwrap();
}
