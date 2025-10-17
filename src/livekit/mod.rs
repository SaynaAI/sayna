//! # LiveKit Integration Module
//!
//! This module provides LiveKit WebRTC integration for the Sayna project, enabling
//! real-time audio streaming and room connectivity. It offers both low-level client
//! access and high-level management interfaces for seamless integration with STT/TTS
//! services.
//!
//! ## Features
//!
//! - **Real-time Audio Streaming**: Receive audio tracks from LiveKit room participants
//! - **WebRTC Connectivity**: Full WebRTC support through LiveKit's Rust SDK
//! - **Audio Format Conversion**: Convert LiveKit audio (i16) to processing format (u8)
//! - **Event-Driven Architecture**: Handle room events, participant changes, track subscriptions
//! - **Connection Management**: Robust connection lifecycle with proper cleanup
//! - **Callback System**: Flexible audio processing callbacks for integration
//! - **High-Level Management**: Simplified interface for common use cases
//!
//! ## Usage Example
//!
//! ```rust,no_run
//! use sayna::livekit::{LiveKitConfig, LiveKitManager};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Configure LiveKit connection
//!     let config = LiveKitConfig {
//!         url: "wss://your-livekit-server.com".to_string(),
//!         token: "your-jwt-token".to_string(),
//!         room_name: "your-room".to_string(),
//!         sample_rate: 24000,
//!         channels: 1,
//!         enable_noise_filter: cfg!(feature = "noise-filter"),
//!         listen_participants: vec![],
//!     };
//!
//!     // Create and initialize manager
//!     let mut manager = LiveKitManager::new();
//!     manager.initialize(config).await?;
//!
//!     // Set up audio processing callback
//!     manager.set_audio_callback(|audio_data: Vec<u8>| {
//!         println!("Received {} bytes of audio", audio_data.len());
//!         // Forward to STT service, save to file, etc.
//!     }).await?;
//!
//!     // Keep connection alive
//!     tokio::time::sleep(std::time::Duration::from_secs(30)).await;
//!
//!     // Disconnect gracefully
//!     manager.disconnect().await?;
//!     Ok(())
//! }
//! ```
//!
//! ## Audio Processing
//!
//! The module handles audio data conversion from LiveKit's native i16 format to u8 byte
//! arrays using little-endian encoding, making it compatible with STT services and other
//! audio processing systems in the Sayna ecosystem.
//!
//! ## Integration with Sayna
//!
//! This module integrates with the unified STT/TTS server architecture by:
//! - Receiving real-time audio from LiveKit participants
//! - Converting audio format for STT processing compatibility
//! - Providing callbacks for forwarding audio to STT services
//! - Supporting the WebSocket API abstraction layer

mod client;
mod manager;
pub mod operations;
pub mod room_handler;
pub mod sip_handler;
mod types;

// Re-export public types and traits
pub use client::{AudioCallback, DataCallback, DataMessage, LiveKitClient};
pub use manager::LiveKitManager;
pub use operations::{LiveKitOperation, OperationQueue};
pub use room_handler::LiveKitRoomHandler;
pub use sip_handler::{DispatchConfig, LiveKitSipHandler, TrunkConfig};
pub use types::{
    AudioFrameInfo, ConnectionStatus, LiveKitConfig, LiveKitError, ParticipantInfo, RoomInfo,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_livekit_manager_creation() {
        let manager = LiveKitManager::new();

        // Should start disconnected
        assert!(!manager.is_connected().await);

        // Should have no participants initially
        assert!(manager.get_participants().await.is_empty());

        // Should have no room info initially
        assert!(manager.get_room_info().await.is_none());
    }

    #[tokio::test]
    async fn test_livekit_config() {
        let config = LiveKitConfig {
            url: "wss://test.example.com".to_string(),
            token: "test-token".to_string(),
            room_name: "test-room".to_string(),
            sample_rate: 24000,
            channels: 1,
            enable_noise_filter: cfg!(feature = "noise-filter"),
            listen_participants: vec![],
        };

        assert_eq!(config.url, "wss://test.example.com");
        assert_eq!(config.token, "test-token");
        assert_eq!(config.enable_noise_filter, cfg!(feature = "noise-filter"));
    }

    #[test]
    fn test_connection_status_enum() {
        use ConnectionStatus::*;

        assert_eq!(Disconnected, Disconnected);
        assert_ne!(Connecting, Connected);
        assert_ne!(Connected, Failed);
    }
}
