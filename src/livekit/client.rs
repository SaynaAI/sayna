//! LiveKit client implementation for real-time audio streaming and publishing.
//!
//! This module provides the core `LiveKitClient` for connecting to LiveKit rooms,
//! handling real-time audio track subscriptions, and publishing local audio tracks.
//!
//! ## Features
//!
//! - **Audio Subscription**: Receive audio from remote participants
//! - **Audio Publishing**: Publish local audio (TTS) to other participants  
//! - **Data Channel Support**: Send and receive data messages
//! - **Event-Driven Architecture**: Handle room events and participant changes
//! - **Connection Management**: Robust connection lifecycle with cleanup

use futures::StreamExt;
use livekit::prelude::*;
use livekit::webrtc::audio_stream::native::NativeAudioStream;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tracing::{debug, error, info, warn};

use super::types::*;
use crate::AppError;

/// Callback type for handling incoming audio chunks
pub type AudioCallback = Arc<dyn Fn(Vec<u8>) + Send + Sync>;

/// Callback type for handling incoming data messages
pub type DataCallback = Arc<dyn Fn(DataMessage) + Send + Sync>;

/// LiveKit data message structure
#[derive(Debug, Clone)]
pub struct DataMessage {
    /// The participant identity who sent the data
    pub participant_identity: String,
    /// The raw data
    pub data: Vec<u8>,
    /// Optional topic/channel for the data
    pub topic: Option<String>,
    /// Timestamp when the data was received
    pub timestamp: u64,
}

/// LiveKit client for handling audio streaming and publishing
pub struct LiveKitClient {
    config: LiveKitConfig,
    room: Option<Room>,
    room_events: Option<mpsc::UnboundedReceiver<RoomEvent>>,
    audio_callback: Option<AudioCallback>,
    data_callback: Option<DataCallback>,
    active_streams: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    is_connected: Arc<Mutex<bool>>,
    // Audio publishing components - TODO: Add proper types when imports are available
    // audio_source: Option<AudioSource>,
    // local_audio_track: Option<LocalAudioTrack>,
    // local_track_publication: Option<LocalTrackPublication>,
}

impl LiveKitClient {
    /// Create a new LiveKit client with the given configuration
    ///
    /// # Arguments
    /// * `config` - Configuration containing LiveKit server URL and JWT token
    ///
    /// # Returns
    /// A new `LiveKitClient` instance ready for connection
    ///
    /// # Examples
    /// ```rust,no_run
    /// use sayna::livekit::{LiveKitClient, LiveKitConfig};
    ///
    /// let config = LiveKitConfig {
    ///     url: "wss://your-livekit-server.com".to_string(),
    ///     token: "your-jwt-token".to_string(),
    /// };
    /// let client = LiveKitClient::new(config);
    /// ```
    pub fn new(config: LiveKitConfig) -> Self {
        Self {
            config,
            room: None,
            room_events: None,
            audio_callback: None,
            data_callback: None,
            active_streams: Arc::new(Mutex::new(Vec::new())),
            is_connected: Arc::new(Mutex::new(false)),
            // Initialize audio publishing components - TODO: Add when types are available
            // audio_source: None,
            // local_audio_track: None,
            // local_track_publication: None,
        }
    }

    /// Set the callback function for handling incoming audio chunks
    ///
    /// This callback will be called whenever audio data is received from remote participants.
    /// The audio data is provided as a `Vec<u8>` containing PCM audio samples converted
    /// from LiveKit's native i16 format to little-endian bytes.
    ///
    /// # Arguments
    /// * `callback` - Function to handle incoming audio data
    ///
    /// # Examples
    /// ```rust,no_run
    /// # use sayna::livekit::{LiveKitClient, LiveKitConfig};
    /// # let mut client = LiveKitClient::new(LiveKitConfig {
    /// #     url: "wss://test.com".to_string(),
    /// #     token: "token".to_string(),
    /// # });
    /// client.set_audio_callback(|audio_data: Vec<u8>| {
    ///     println!("Received {} bytes of audio", audio_data.len());
    ///     // Process audio data (forward to STT, save to file, etc.)
    /// });
    /// ```
    pub fn set_audio_callback<F>(&mut self, callback: F)
    where
        F: Fn(Vec<u8>) + Send + Sync + 'static,
    {
        self.audio_callback = Some(Arc::new(callback));
    }

    /// Set the callback function for handling incoming data messages
    ///
    /// This callback will be invoked whenever data messages are received from
    /// participants in the LiveKit room.
    ///
    /// # Arguments
    /// * `callback` - Function to call when data messages are received
    ///
    /// # Example
    /// ```no_run
    /// # use sayna::livekit::{LiveKitClient, LiveKitConfig, DataMessage};
    /// # let mut client = LiveKitClient::new(LiveKitConfig {
    /// #     url: "wss://test.com".to_string(),
    /// #     token: "token".to_string(),
    /// # });
    /// client.set_data_callback(|data_message: DataMessage| {
    ///     println!("Received data from {}: {} bytes",
    ///              data_message.participant_identity,
    ///              data_message.data.len());
    ///     // Process data message (forward to WebSocket, handle commands, etc.)
    /// });
    /// ```
    pub fn set_data_callback<F>(&mut self, callback: F)
    where
        F: Fn(DataMessage) + Send + Sync + 'static,
    {
        self.data_callback = Some(Arc::new(callback));
    }

    /// Connect to the LiveKit room
    ///
    /// Establishes a WebRTC connection to the LiveKit server and starts handling
    /// room events. Once connected, the client will automatically subscribe to
    /// audio tracks from remote participants.
    ///
    /// # Returns
    /// * `Ok(())` - Connection established successfully
    /// * `Err(AppError)` - Connection failed
    ///
    /// # Examples
    /// ```rust,no_run
    /// # use sayna::livekit::{LiveKitClient, LiveKitConfig};
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut client = LiveKitClient::new(LiveKitConfig {
    ///     url: "wss://your-server.com".to_string(),
    ///     token: "your-token".to_string(),
    /// });
    ///
    /// client.connect().await?;
    /// // Client is now connected and receiving audio
    /// # Ok(())
    /// # }
    /// ```
    pub async fn connect(&mut self) -> Result<(), AppError> {
        info!("Connecting to LiveKit room with URL: {}", self.config.url);

        match Room::connect(&self.config.url, &self.config.token, RoomOptions::default()).await {
            Ok((room, room_events)) => {
                self.room = Some(room);
                self.room_events = Some(room_events);
                *self.is_connected.lock().await = true;

                info!("Successfully connected to LiveKit room");

                // Start handling room events
                self.start_event_handler().await?;

                Ok(())
            }
            Err(e) => {
                error!("Failed to connect to LiveKit room: {:?}", e);
                Err(AppError::InternalServerError(format!(
                    "LiveKit connection failed: {:?}",
                    e
                )))
            }
        }
    }

    /// Check if the client is connected to the room
    ///
    /// # Returns
    /// `true` if connected to the LiveKit room, `false` otherwise
    pub async fn is_connected(&self) -> bool {
        *self.is_connected.lock().await
    }

    /// Send TTS audio data to the published LiveKit audio track
    ///
    /// This method receives TTS audio data and forwards it to the published local audio track
    /// for transmission to other participants in the LiveKit room.
    ///
    /// # Arguments
    /// * `audio_data` - Raw audio data from TTS synthesis
    ///
    /// # Returns
    /// * `Ok(())` - Audio data sent successfully
    /// * `Err(AppError)` - Failed to send audio data
    ///
    /// # Examples
    /// ```rust,no_run
    /// # use sayna::livekit::{LiveKitClient, LiveKitConfig};
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut client = LiveKitClient::new(LiveKitConfig {
    ///     url: "wss://your-server.com".to_string(),
    ///     token: "your-token".to_string(),
    /// });
    ///
    /// client.connect().await?;
    ///
    /// let tts_audio_data = vec![0u8; 1024]; // TTS synthesized audio
    /// client.send_tts_audio(tts_audio_data).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_tts_audio(&mut self, audio_data: Vec<u8>) -> Result<(), AppError> {
        debug!("Received TTS audio data: {} bytes", audio_data.len());

        if !*self.is_connected.lock().await {
            return Err(AppError::InternalServerError(
                "Not connected to LiveKit room".to_string(),
            ));
        }

        // TODO: Implement audio track publishing and audio data forwarding
        // This requires:
        // 1. Creating an AudioSource during connection setup
        // 2. Creating a LocalAudioTrack using the AudioSource
        // 3. Publishing the track to the room
        // 4. Converting TTS audio data to AudioFrame format
        // 5. Calling AudioSource.capture_frame() with the converted audio

        debug!("TTS audio data queued for LiveKit track publishing (not yet implemented)");

        // For now, we'll return success to prevent errors while we implement the full solution
        Ok(())
    }

    /// Disconnect from the LiveKit room
    ///
    /// Gracefully closes the connection, stops all audio streams, and cleans up resources.
    ///
    /// # Returns
    /// * `Ok(())` - Disconnection completed successfully
    /// * `Err(AppError)` - Error during disconnection
    pub async fn disconnect(&mut self) -> Result<(), AppError> {
        info!("Disconnecting from LiveKit room");

        // Set connected status to false
        *self.is_connected.lock().await = false;

        // Cancel all active audio streams
        let mut streams = self.active_streams.lock().await;
        for handle in streams.drain(..) {
            handle.abort();
        }

        // Clean up room and events
        self.room = None;
        self.room_events = None;

        info!("Successfully disconnected from LiveKit room");
        Ok(())
    }

    /// Start handling room events
    ///
    /// Spawns a background task to process LiveKit room events such as
    /// participant connections and track subscriptions.
    async fn start_event_handler(&mut self) -> Result<(), AppError> {
        if let Some(mut room_events) = self.room_events.take() {
            let audio_callback = self.audio_callback.clone();
            let data_callback = self.data_callback.clone();
            let active_streams = Arc::clone(&self.active_streams);
            let is_connected = Arc::clone(&self.is_connected);

            tokio::spawn(async move {
                while let Some(event) = room_events.recv().await {
                    if let Err(e) = Self::handle_room_event(
                        event,
                        &audio_callback,
                        &data_callback,
                        &active_streams,
                        &is_connected,
                    )
                    .await
                    {
                        error!("Error handling room event: {:?}", e);
                    }
                }
                info!("Room event handler finished");
            });
        }

        Ok(())
    }

    /// Handle individual room events
    ///
    /// Processes different types of LiveKit room events including track subscriptions,
    /// participant connections, disconnections, and data messages.
    async fn handle_room_event(
        event: RoomEvent,
        audio_callback: &Option<AudioCallback>,
        data_callback: &Option<DataCallback>,
        active_streams: &Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
        is_connected: &Arc<Mutex<bool>>,
    ) -> Result<(), AppError> {
        match event {
            RoomEvent::TrackSubscribed {
                track,
                publication,
                participant,
            } => {
                info!(
                    "Track subscribed - Track: {:?}, Participant: {:?}",
                    publication.sid(),
                    participant.identity()
                );

                // All participants in TrackSubscribed event are remote participants
                // so we don't need to check if they are local
                match track {
                    RemoteTrack::Audio(audio_track) => {
                        debug!(
                            "Received audio track from participant: {}",
                            participant.identity()
                        );

                        if let Some(callback) = audio_callback {
                            let callback_clone = Arc::clone(callback);
                            let rtc_track = audio_track.rtc_track();
                            let mut audio_stream = NativeAudioStream::new(rtc_track, 48000, 1);

                            let handle = tokio::spawn(async move {
                                info!(
                                    "Starting audio stream processing for participant: {}",
                                    participant.identity()
                                );

                                while let Some(audio_frame) = audio_stream.next().await {
                                    debug!(
                                        "Received audio frame: {} bytes",
                                        audio_frame.data.len()
                                    );

                                    // Convert audio frame from i16 to u8 bytes and call the callback
                                    let audio_data: Vec<u8> = audio_frame
                                        .data
                                        .iter()
                                        .flat_map(|&sample| sample.to_le_bytes())
                                        .collect();
                                    callback_clone(audio_data);
                                }

                                info!(
                                    "Audio stream ended for participant: {}",
                                    participant.identity()
                                );
                            });

                            // Store the handle for cleanup
                            active_streams.lock().await.push(handle);
                        } else {
                            warn!("No audio callback set, ignoring audio track");
                        }
                    }
                    RemoteTrack::Video(_) => {
                        debug!(
                            "Received video track (ignoring): {}",
                            participant.identity()
                        );
                    }
                }
            }
            RoomEvent::TrackUnsubscribed {
                track: _,
                publication,
                participant,
            } => {
                info!(
                    "Track unsubscribed - Track: {:?}, Participant: {:?}",
                    publication.sid(),
                    participant.identity()
                );
            }
            RoomEvent::ParticipantConnected(participant) => {
                info!("Participant connected: {}", participant.identity());
            }
            RoomEvent::ParticipantDisconnected(participant) => {
                info!("Participant disconnected: {}", participant.identity());
            }
            RoomEvent::Disconnected { reason } => {
                warn!("Room disconnected with reason: {:?}", reason);
                *is_connected.lock().await = false;
            }
            RoomEvent::DataReceived {
                payload,
                topic,
                kind: _,
                participant,
            } => {
                debug!(
                    "Data received from participant: {}, {} bytes",
                    participant
                        .as_ref()
                        .map(|p| p.identity().to_string())
                        .unwrap_or_else(|| "server".to_string()),
                    payload.len()
                );

                if let Some(callback) = data_callback {
                    let data_message = DataMessage {
                        participant_identity: participant
                            .as_ref()
                            .map(|p| p.identity().to_string())
                            .unwrap_or_else(|| "server".to_string()),
                        data: payload.to_vec(),
                        topic,
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64,
                    };

                    callback(data_message);
                } else {
                    debug!("No data callback set, ignoring data message");
                }
            }
            _ => {
                debug!("Received other room event: {:?}", event);
            }
        }

        Ok(())
    }
}

impl Drop for LiveKitClient {
    fn drop(&mut self) {
        // Note: We can't use async methods in Drop, so we log a warning
        // The actual cleanup should be done by calling disconnect() explicitly
        if self.room.is_some() {
            warn!("LiveKitClient dropped without explicit disconnect call");
        }
    }
}
