use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex, RwLock, mpsc};
use tracing::{debug, error, info, warn};

use super::client::LiveKitClient;
use super::types::{ConnectionStatus, LiveKitConfig, ParticipantInfo, RoomInfo};
use crate::AppError;

/// High-level manager for LiveKit connections
///
/// Optimized for low latency with atomic operations and minimal locking
pub struct LiveKitManager {
    client: Arc<Mutex<Option<LiveKitClient>>>,
    connection_status: Arc<RwLock<ConnectionStatus>>,
    audio_sender: Option<mpsc::UnboundedSender<Vec<u8>>>,
    participants: Arc<RwLock<Vec<ParticipantInfo>>>,
    room_info: Arc<RwLock<Option<RoomInfo>>>,
    // Use atomic bool for fast connection status checks
    is_connected_atomic: Arc<AtomicBool>,
}

impl LiveKitManager {
    /// Create a new LiveKit manager
    pub fn new() -> Self {
        Self {
            client: Arc::new(Mutex::new(None)),
            connection_status: Arc::new(RwLock::new(ConnectionStatus::Disconnected)),
            audio_sender: None,
            participants: Arc::new(RwLock::new(Vec::new())),
            room_info: Arc::new(RwLock::new(None)),
            is_connected_atomic: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Initialize the LiveKit connection with configuration
    pub async fn initialize(&mut self, config: LiveKitConfig) -> Result<(), AppError> {
        info!("Initializing LiveKit manager with config: {:?}", config);

        // Update connection status
        *self.connection_status.write().await = ConnectionStatus::Connecting;

        // Create new client
        let mut client = LiveKitClient::new(config);

        // Set up audio channel for forwarding audio data
        let (audio_tx, _audio_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        self.audio_sender = Some(audio_tx.clone());

        // Set audio callback to forward audio data through the channel
        client.set_audio_callback(move |audio_data: Vec<u8>| {
            if let Err(e) = audio_tx.send(audio_data) {
                error!("Failed to forward audio data: {}", e);
            }
        });

        // Connect to LiveKit
        match client.connect().await {
            Ok(()) => {
                *self.connection_status.write().await = ConnectionStatus::Connected;
                self.is_connected_atomic.store(true, Ordering::Release);
                *self.client.lock().await = Some(client);

                info!("LiveKit manager initialized successfully");
                Ok(())
            }
            Err(e) => {
                *self.connection_status.write().await = ConnectionStatus::Failed;
                self.is_connected_atomic.store(false, Ordering::Release);
                error!("Failed to initialize LiveKit manager: {:?}", e);
                Err(e)
            }
        }
    }

    /// Set up an audio callback that will be called with incoming audio data
    pub async fn set_audio_callback<F>(&self, callback: F) -> Result<(), AppError>
    where
        F: Fn(Vec<u8>) + Send + Sync + 'static,
    {
        // Note: This is a placeholder implementation
        // In a real implementation, you'd want a proper way to handle multiple audio callbacks
        if let Some(_audio_sender) = &self.audio_sender {
            let _callback = Arc::new(callback);
            // TODO: Implement proper audio callback handling for multiple subscribers
            warn!(
                "Audio callback set but not fully implemented - requires proper subscriber management"
            );
        }

        Ok(())
    }

    /// Get current connection status
    pub async fn get_connection_status(&self) -> ConnectionStatus {
        self.connection_status.read().await.clone()
    }

    /// Check if connected to LiveKit room
    ///
    /// Uses atomic operations for fast lock-free checking
    #[inline]
    pub async fn is_connected(&self) -> bool {
        // Fast path: check atomic bool first
        if !self.is_connected_atomic.load(Ordering::Acquire) {
            return false;
        }

        // Slow path: verify with actual client (only if atomic says connected)
        if let Some(client) = self.client.lock().await.as_ref() {
            client.is_connected()
        } else {
            false
        }
    }

    /// Get current participants in the room
    pub async fn get_participants(&self) -> Vec<ParticipantInfo> {
        self.participants.read().await.clone()
    }

    /// Get current room information
    pub async fn get_room_info(&self) -> Option<RoomInfo> {
        self.room_info.read().await.clone()
    }

    /// Disconnect from LiveKit room
    pub async fn disconnect(&mut self) -> Result<(), AppError> {
        info!("Disconnecting from LiveKit room");

        // Update atomic flag first for fast path
        self.is_connected_atomic.store(false, Ordering::Release);
        *self.connection_status.write().await = ConnectionStatus::Disconnected;

        if let Some(mut client) = self.client.lock().await.take() {
            match client.disconnect().await {
                Ok(()) => {
                    info!("Successfully disconnected from LiveKit room");
                    // Clear participants and room info
                    self.participants.write().await.clear();
                    *self.room_info.write().await = None;
                    Ok(())
                }
                Err(e) => {
                    error!("Error during LiveKit disconnection: {:?}", e);
                    Err(e)
                }
            }
        } else {
            warn!("No active LiveKit client to disconnect");
            Ok(())
        }
    }

    /// Clear all pending buffered audio from the published LiveKit track
    ///
    /// This method clears any audio data that has been queued for publishing but hasn't
    /// been transmitted yet in the LiveKit audio track. This is useful for stopping TTS
    /// playback immediately when a clear command is received from WebSocket clients.
    ///
    /// This method is specifically designed to be called from the WebSocket handler
    /// during a Clear command to stop all buffered audio that has not been played out yet.
    ///
    /// # Returns
    /// * `Ok(())` - Audio buffer cleared successfully
    /// * `Err(AppError)` - Failed to clear audio buffer or no active client
    ///
    /// # Examples
    /// ```rust,no_run
    /// # use sayna::livekit::{LiveKitManager, LiveKitConfig};
    /// # use std::sync::Arc;
    /// # use tokio;
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = LiveKitConfig {
    ///     url: "wss://your-server.com".to_string(),
    ///     token: "your-token".to_string(),
    ///     room_name: "your-room".to_string(),
    ///     sample_rate: 24000,
    ///     channels: 1,
    ///     enable_noise_filter: true,
    /// };
    ///
    /// let mut manager = LiveKitManager::new();
    /// manager.initialize(config).await?;
    ///
    /// // Clear any pending buffered audio during WebSocket Clear command handling
    /// manager.clear_audio().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn clear_audio(&mut self) -> Result<(), AppError> {
        debug!("Clearing pending buffered audio from LiveKit track");

        // Fast path: check if connected using atomic
        if !self.is_connected_atomic.load(Ordering::Acquire) {
            debug!("Not connected to LiveKit, skipping clear audio");
            return Ok(());
        }

        // Use as_mut() instead of take() to avoid unnecessary moves
        let mut client_guard = self.client.lock().await;
        if let Some(client) = client_guard.as_mut() {
            // Clear the audio buffer on the client
            match client.clear_audio().await {
                Ok(()) => {
                    debug!("Successfully cleared LiveKit audio buffer");
                    Ok(())
                }
                Err(e) => {
                    error!("Failed to clear LiveKit audio buffer: {:?}", e);
                    Err(e)
                }
            }
        } else {
            debug!("No active LiveKit client to clear audio buffer from");
            Ok(())
        }
    }
}

impl Default for LiveKitManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for LiveKitManager {
    fn drop(&mut self) {
        // Note: We can't use async methods in Drop
        warn!("LiveKitManager dropped - ensure disconnect() was called explicitly");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{Duration, timeout};

    #[tokio::test]
    async fn test_livekit_manager_creation() {
        let manager = LiveKitManager::new();
        assert!(!manager.is_connected().await);
    }

    #[tokio::test]
    async fn test_livekit_manager_clear_audio_without_client() {
        let mut manager = LiveKitManager::new();

        // Should not fail when no client is present
        let result = manager.clear_audio().await;
        assert!(
            result.is_ok(),
            "clear_audio should succeed even without client"
        );
    }

    #[tokio::test]
    async fn test_livekit_manager_clear_audio_with_mock_config() {
        let mut manager = LiveKitManager::new();

        // Create a mock configuration
        let config = LiveKitConfig {
            url: "wss://test-server.com".to_string(),
            token: "mock-jwt-token".to_string(),
            room_name: "test-room".to_string(),
            sample_rate: 24000,
            channels: 1,
            enable_noise_filter: true,
        };

        // Try to initialize (this will fail with mock config, but we can still test the clear_audio logic)
        let _init_result = manager.initialize(config).await;

        // Clear audio should handle the case gracefully
        let result = manager.clear_audio().await;
        assert!(
            result.is_ok(),
            "clear_audio should handle invalid client gracefully"
        );
    }

    #[tokio::test]
    async fn test_livekit_manager_audio_callback_registration() {
        let manager = LiveKitManager::new();

        // Test setting audio callback without crashing
        let result = manager
            .set_audio_callback(|_audio_data| {
                // Mock callback
            })
            .await;

        // Should succeed even without active connection
        assert!(result.is_ok(), "Audio callback registration should succeed");
    }

    #[tokio::test]
    async fn test_livekit_manager_connection_status_flow() {
        let mut manager = LiveKitManager::new();

        // Initially not connected
        assert!(!manager.is_connected().await);

        // Create mock config
        let config = LiveKitConfig {
            url: "wss://test-server.com".to_string(),
            token: "mock-jwt-token".to_string(),
            room_name: "test-room".to_string(),
            sample_rate: 24000,
            channels: 1,
            enable_noise_filter: true,
        };

        // Try initialization (will fail with mock config)
        let _init_result = manager.initialize(config).await;

        // Depending on implementation, this might fail or succeed
        // The important thing is that clear_audio works regardless
        let clear_result = manager.clear_audio().await;
        assert!(
            clear_result.is_ok(),
            "clear_audio should work regardless of connection state"
        );

        // Disconnect should also work
        let disconnect_result = manager.disconnect().await;
        assert!(
            disconnect_result.is_ok(),
            "disconnect should work regardless of connection state"
        );
    }

    #[tokio::test]
    async fn test_livekit_manager_clear_audio_timing() {
        let mut manager = LiveKitManager::new();

        // Test that clear_audio completes within reasonable time
        let result = timeout(Duration::from_millis(500), manager.clear_audio()).await;

        assert!(result.is_ok(), "clear_audio should complete within 500ms");
        assert!(result.unwrap().is_ok(), "clear_audio should succeed");
    }

    #[tokio::test]
    async fn test_livekit_manager_multiple_clear_audio_calls() {
        let mut manager = LiveKitManager::new();

        // Test multiple clear_audio calls in succession
        for i in 0..5 {
            let result = manager.clear_audio().await;
            assert!(result.is_ok(), "clear_audio call {} should succeed", i + 1);
        }
    }

    #[tokio::test]
    async fn test_livekit_manager_concurrent_clear_audio() {
        let manager = Arc::new(Mutex::new(LiveKitManager::new()));

        // Test concurrent clear_audio calls
        let mut handles = Vec::new();

        for i in 0..3 {
            let manager_clone = manager.clone();
            let handle = tokio::spawn(async move {
                let result = manager_clone.lock().await.clear_audio().await;
                assert!(
                    result.is_ok(),
                    "Concurrent clear_audio call {} should succeed",
                    i + 1
                );
            });
            handles.push(handle);
        }

        // Wait for all concurrent calls to complete
        for handle in handles {
            handle
                .await
                .expect("Concurrent clear_audio task should complete");
        }
    }

    #[tokio::test]
    async fn test_livekit_manager_clear_audio_after_disconnect() {
        let mut manager = LiveKitManager::new();

        // Disconnect first
        let disconnect_result = manager.disconnect().await;
        assert!(disconnect_result.is_ok(), "Disconnect should succeed");

        // Then try clear_audio
        let clear_result = manager.clear_audio().await;
        assert!(
            clear_result.is_ok(),
            "clear_audio should work after disconnect"
        );
    }

    #[tokio::test]
    async fn test_livekit_manager_clear_audio_error_recovery() {
        let mut manager = LiveKitManager::new();

        // Try clear_audio multiple times to ensure it doesn't leave the manager in a bad state
        for i in 0..3 {
            let result = manager.clear_audio().await;
            assert!(
                result.is_ok(),
                "clear_audio iteration {} should succeed",
                i + 1
            );

            // Verify manager is still in a usable state
            assert!(
                !manager.is_connected().await,
                "Manager should maintain consistent state"
            );
        }
    }

    #[tokio::test]
    async fn test_livekit_manager_integration_with_voice_manager_pattern() {
        // This test mimics how clear_audio would be used in conjunction with voice manager clearing
        let mut manager = LiveKitManager::new();

        // Simulate the pattern used in handle_clear_message
        // 1. Clear TTS (voice manager) - simulated
        let tts_clear_success = true;

        // 2. Clear LiveKit audio buffer
        let livekit_clear_result = manager.clear_audio().await;

        // Both should succeed independently
        assert!(tts_clear_success, "TTS clear should succeed (simulated)");
        assert!(livekit_clear_result.is_ok(), "LiveKit clear should succeed");

        // The WebSocket handler pattern should continue even if one fails
        let combined_success = tts_clear_success && livekit_clear_result.is_ok();
        assert!(combined_success, "Combined clear operation should succeed");
    }
}
