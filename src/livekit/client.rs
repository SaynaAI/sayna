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
use livekit::{
    options::TrackPublishOptions,
    track::{LocalAudioTrack, LocalTrack, TrackSource},
    webrtc::{
        audio_source::native::NativeAudioSource,
        audio_stream::native::NativeAudioStream,
        prelude::{AudioFrame, AudioSourceOptions, RtcAudioSource},
    },
};
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tracing::{debug, error, info, warn};

use super::types::*;
use crate::AppError;

/// Callback type for handling incoming audio chunks
pub type AudioCallback = Arc<dyn Fn(Vec<u8>) + Send + Sync>;

/// Callback type for handling incoming data messages
pub type DataCallback = Arc<dyn Fn(DataMessage) + Send + Sync>;

/// Callback type for handling participant disconnection events
pub type ParticipantDisconnectCallback = Arc<dyn Fn(ParticipantDisconnectEvent) + Send + Sync>;

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

/// LiveKit participant disconnect event structure
#[derive(Debug, Clone)]
pub struct ParticipantDisconnectEvent {
    /// The participant identity who disconnected
    pub participant_identity: String,
    /// The participant's display name (if available)
    pub participant_name: Option<String>,
    /// Room identifier
    pub room_name: String,
    /// Timestamp when the disconnection occurred
    pub timestamp: u64,
}

/// LiveKit client for handling audio streaming and publishing
pub struct LiveKitClient {
    config: LiveKitConfig,
    room: Option<Room>,
    room_events: Option<mpsc::UnboundedReceiver<RoomEvent>>,
    audio_callback: Option<AudioCallback>,
    data_callback: Option<DataCallback>,
    participant_disconnect_callback: Option<ParticipantDisconnectCallback>,
    active_streams: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    is_connected: Arc<Mutex<bool>>,
    // Audio publishing components
    audio_source: Option<Arc<NativeAudioSource>>,
    local_audio_track: Option<LocalAudioTrack>,
    local_track_publication: Option<LocalTrackPublication>,
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
    ///     sample_rate: 24000,
    ///     channels: 1,
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
            participant_disconnect_callback: None,
            active_streams: Arc::new(Mutex::new(Vec::new())),
            is_connected: Arc::new(Mutex::new(false)),
            // Initialize audio publishing components
            audio_source: None,
            local_audio_track: None,
            local_track_publication: None,
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
    /// #     sample_rate: 24000,
    /// #     channels: 1,
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
    /// #     sample_rate: 24000,
    /// #     channels: 1,
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

    /// Set a callback to handle participant disconnection events
    ///
    /// This callback will be called whenever a participant leaves the LiveKit room.
    /// The callback receives information about the disconnected participant including
    /// their identity, display name (if available), and the timestamp of disconnection.
    ///
    /// # Arguments
    /// * `callback` - Function to handle participant disconnect events
    ///
    /// # Examples
    /// ```rust,no_run
    /// # use sayna::livekit::{LiveKitClient, LiveKitConfig};
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut client = LiveKitClient::new(LiveKitConfig {
    ///     url: "wss://your-server.com".to_string(),
    ///     token: "your-token".to_string(),
    ///     sample_rate: 24000,
    ///     channels: 1,
    /// });
    ///
    /// client.set_participant_disconnect_callback(|event| {
    ///     println!("Participant {} left the room", event.participant_identity);
    /// });
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_participant_disconnect_callback<F>(&mut self, callback: F)
    where
        F: Fn(ParticipantDisconnectEvent) + Send + Sync + 'static,
    {
        self.participant_disconnect_callback = Some(Arc::new(callback));
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
    ///     sample_rate: 24000,
    ///     channels: 1,
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

                // Set up audio source and track for publishing TTS audio
                self.setup_audio_publishing().await?;

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

    /// Set up audio source and track for publishing TTS audio
    ///
    /// This method creates a NativeAudioSource and LocalAudioTrack for publishing
    /// TTS audio to other participants in the LiveKit room, following the pattern
    /// from the LiveKit Rust SDK examples.
    ///
    /// # Returns
    /// * `Ok(())` - Audio publishing setup successful
    /// * `Err(AppError)` - Setup failed
    async fn setup_audio_publishing(&mut self) -> Result<(), AppError> {
        info!(
            "Setting up audio publishing: {}Hz, {} channels",
            self.config.sample_rate, self.config.channels
        );

        // Create audio source options (disable processing for TTS audio)
        let audio_source_options = AudioSourceOptions {
            echo_cancellation: false,
            noise_suppression: false,
            auto_gain_control: false,
        };

        // Calculate samples per frame (10ms frames are common)
        let samples_per_frame = (self.config.sample_rate * 10) / 1000; // 10ms worth of samples

        // Create native audio source - following LiveKit examples pattern
        let audio_source = Arc::new(NativeAudioSource::new(
            audio_source_options,
            self.config.sample_rate,
            self.config.channels as u32,
            samples_per_frame,
        ));

        // Create RTC audio source from native audio source
        // Based on LiveKit Rust SDK patterns, RtcAudioSource likely wraps NativeAudioSource
        let rtc_audio_source = RtcAudioSource::Native((*audio_source).clone());

        // Create local audio track from the RTC audio source
        let local_audio_track = LocalAudioTrack::create_audio_track("tts-audio", rtc_audio_source);

        // Publish the track to the room
        if let Some(room) = &self.room {
            let publish_options = TrackPublishOptions {
                source: TrackSource::Microphone,
                ..Default::default()
            };

            match room
                .local_participant()
                .publish_track(
                    LocalTrack::Audio(local_audio_track.clone()),
                    publish_options,
                )
                .await
            {
                Ok(publication) => {
                    info!(
                        "Successfully published TTS audio track: {}",
                        publication.sid()
                    );

                    // Store the components for later use
                    self.audio_source = Some(audio_source);
                    self.local_audio_track = Some(local_audio_track);
                    self.local_track_publication = Some(publication);
                }
                Err(e) => {
                    error!("Failed to publish audio track: {:?}", e);
                    return Err(AppError::InternalServerError(format!(
                        "Failed to publish audio track: {:?}",
                        e
                    )));
                }
            }
        } else {
            return Err(AppError::InternalServerError(
                "Room not available for track publishing".to_string(),
            ));
        }

        info!("Audio publishing setup completed successfully");
        Ok(())
    }

    /// Send TTS audio data to the published LiveKit audio track
    ///
    /// This method receives TTS audio data and forwards it to the published local audio track
    /// for transmission to other participants in the LiveKit room.
    ///
    /// # Arguments
    /// * `audio_data` - Raw audio data from TTS synthesis (PCM bytes)
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
    ///     sample_rate: 24000,
    ///     channels: 1,
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

        // Check if we have an audio source available
        if let Some(audio_source) = &self.audio_source {
            // Convert TTS audio data to AudioFrame
            let audio_frame = self.convert_tts_to_audio_frame(audio_data)?;

            // Send the audio frame to the published track
            match audio_source.capture_frame(&audio_frame).await {
                Ok(()) => {
                    debug!("Successfully sent TTS audio frame to LiveKit track");
                }
                Err(e) => {
                    error!("Failed to capture audio frame: {:?}", e);
                    return Err(AppError::InternalServerError(format!(
                        "Failed to capture audio frame: {:?}",
                        e
                    )));
                }
            }
        } else {
            warn!("Audio source not available - TTS audio cannot be published to LiveKit");
            // For now, we'll return success to prevent errors in the WebSocket handler
            // This allows the system to work with WebSocket-only audio routing
        }

        Ok(())
    }

    /// Convert TTS audio data to LiveKit AudioFrame
    ///
    /// This method converts raw audio bytes from TTS synthesis to the AudioFrame format
    /// expected by LiveKit's audio source.
    ///
    /// # Arguments
    /// * `audio_data` - Raw audio bytes (typically PCM format)
    ///
    /// # Returns
    /// * `Ok(AudioFrame)` - Converted audio frame
    /// * `Err(AppError)` - Conversion failed
    fn convert_tts_to_audio_frame(&self, audio_data: Vec<u8>) -> Result<AudioFrame, AppError> {
        // Convert bytes to i16 samples (assuming little-endian PCM)
        if audio_data.len() % 2 != 0 {
            return Err(AppError::InternalServerError(
                "Audio data length must be even for 16-bit samples".to_string(),
            ));
        }

        let samples: Vec<i16> = audio_data
            .chunks_exact(2)
            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();

        let samples_per_channel = samples.len() / self.config.channels as usize;

        // Create AudioFrame
        let audio_frame = AudioFrame {
            data: samples.into(),
            sample_rate: self.config.sample_rate,
            num_channels: self.config.channels as u32,
            samples_per_channel: samples_per_channel as u32,
        };

        Ok(audio_frame)
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

        // Clean up audio publishing components
        self.audio_source = None;
        self.local_audio_track = None;
        self.local_track_publication = None;

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
            let participant_disconnect_callback = self.participant_disconnect_callback.clone();
            let active_streams = Arc::clone(&self.active_streams);
            let is_connected = Arc::clone(&self.is_connected);
            let config = self.config.clone();

            tokio::spawn(async move {
                while let Some(event) = room_events.recv().await {
                    if let Err(e) = Self::handle_room_event(
                        event,
                        &audio_callback,
                        &data_callback,
                        &participant_disconnect_callback,
                        &active_streams,
                        &is_connected,
                        &config,
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
        participant_disconnect_callback: &Option<ParticipantDisconnectCallback>,
        active_streams: &Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
        is_connected: &Arc<Mutex<bool>>,
        config: &LiveKitConfig,
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
                            let mut audio_stream = NativeAudioStream::new(
                                rtc_track,
                                config.sample_rate as i32,
                                config.channels as i32,
                            );

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

                // Trigger participant disconnect callback if set
                if let Some(callback) = participant_disconnect_callback {
                    let event = ParticipantDisconnectEvent {
                        participant_identity: participant.identity().to_string(),
                        participant_name: Some(participant.name().to_string()),
                        room_name: "livekit".to_string(), // TODO: Get actual room name from Room object
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64,
                    };

                    callback(event);
                }
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
