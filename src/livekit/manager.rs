use std::sync::Arc;
use tokio::sync::{Mutex, RwLock, mpsc};
use tracing::{debug, error, info, warn};

use super::client::LiveKitClient;
use super::types::{ConnectionStatus, LiveKitConfig, ParticipantInfo, RoomInfo};
use crate::AppError;

/// High-level manager for LiveKit connections
pub struct LiveKitManager {
    client: Arc<Mutex<Option<LiveKitClient>>>,
    connection_status: Arc<RwLock<ConnectionStatus>>,
    audio_sender: Option<mpsc::UnboundedSender<Vec<u8>>>,
    participants: Arc<RwLock<Vec<ParticipantInfo>>>,
    room_info: Arc<RwLock<Option<RoomInfo>>>,
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
                *self.client.lock().await = Some(client);

                info!("LiveKit manager initialized successfully");
                Ok(())
            }
            Err(e) => {
                *self.connection_status.write().await = ConnectionStatus::Failed;
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
    pub async fn is_connected(&self) -> bool {
        if let Some(client) = self.client.lock().await.as_ref() {
            client.is_connected().await
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

    /// Get a channel receiver for audio data
    pub async fn get_audio_receiver(&self) -> Option<mpsc::UnboundedReceiver<Vec<u8>>> {
        if let Some(_sender) = &self.audio_sender {
            let (_tx, rx) = mpsc::unbounded_channel();

            // TODO: Implement proper audio receiver bridging
            // This is a simplified approach - in a real implementation,
            // you'd want a more sophisticated way to handle multiple receivers
            debug!("Audio receiver requested but not fully implemented");

            Some(rx)
        } else {
            None
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
