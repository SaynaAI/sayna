use futures::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

use sayna::{ServerConfig, routes, state::AppState};

#[tokio::test]
async fn test_websocket_echo() {
    // Create test config
    let config = ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 0, // Let the OS assign a port
    };

    // Create application state
    let app_state = AppState::new(config.clone());

    // Create router
    let app = routes::api::create_api_router()
        .merge(routes::ws::create_ws_router())
        .with_state(app_state);

    // Create listener
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
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

    // Send test message
    let test_message = "Hello, WebSocket!";
    write
        .send(Message::Text(test_message.into()))
        .await
        .unwrap();

    // Receive echo
    let response = read.next().await.unwrap().unwrap();

    // Assert echo message
    match response {
        Message::Text(text) => {
            assert_eq!(text, test_message);
        }
        _ => panic!("Expected text message"),
    }

    // Test binary echo
    let test_binary = vec![1, 2, 3, 4, 5];
    write
        .send(Message::Binary(test_binary.clone().into()))
        .await
        .unwrap();

    // Receive binary echo
    let response = read.next().await.unwrap().unwrap();

    // Assert binary echo
    match response {
        Message::Binary(data) => {
            assert_eq!(data.as_ref(), test_binary.as_slice());
        }
        _ => panic!("Expected binary message"),
    }

    // Close connection
    write.close().await.unwrap();
}
