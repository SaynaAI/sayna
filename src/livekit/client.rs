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
use serde_json::json;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tracing::{debug, error, info, warn};

use super::types::*;
use crate::{AppError, utils::noise_filter::reduce_noise_async};

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
                    "LiveKit connection failed: {e:?}"
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
                        "Failed to publish audio track: {e:?}"
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
                        "Failed to capture audio frame: {e:?}"
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

    /// Send a data message to the LiveKit room
    ///
    /// This method publishes a JSON data message to the specified topic in the LiveKit room.
    /// The message is serialized using serde_json::json!() and then converted to bytes.
    ///
    /// # Arguments
    /// * `topic` - The topic/channel to publish the data to
    /// * `data` - The data to send, typically a JSON object
    ///
    /// # Returns
    /// * `Ok(())` - Data message sent successfully
    /// * `Err(AppError)` - Failed to send data message
    ///
    /// # Examples
    /// ```rust,no_run
    /// # use sayna::livekit::{LiveKitClient, LiveKitConfig};
    /// # use serde_json::json;
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
    /// let data_message = json!({
    ///     "type": "command",
    ///     "command": "start_tts",
    ///     "text": "Hello, LiveKit!"
    /// });
    /// client.send_data_message("tts-commands", data_message).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_data_message(
        &mut self,
        topic: &str,
        data: serde_json::Value,
    ) -> Result<(), AppError> {
        debug!("Sending data message to topic: {}, data: {:?}", topic, data);

        if !*self.is_connected.lock().await {
            return Err(AppError::InternalServerError(
                "Not connected to LiveKit room".to_string(),
            ));
        }

        let serialized_data = serde_json::to_vec(&data).map_err(|e| {
            AppError::InternalServerError(format!("Failed to serialize JSON data: {e}"))
        })?;

        if let Some(room) = &self.room {
            let data_packet = DataPacket {
                payload: serialized_data,
                topic: Some(topic.to_string()),
                ..Default::default()
            };
            match room.local_participant().publish_data(data_packet).await {
                Ok(_) => {
                    debug!("Successfully sent data message to topic: {}", topic);
                }
                Err(e) => {
                    error!("Failed to send data message to topic {}: {:?}", topic, e);
                    return Err(AppError::InternalServerError(format!(
                        "Failed to send data message to topic {topic}: {e:?}"
                    )));
                }
            }
        } else {
            return Err(AppError::InternalServerError(
                "Room not available for data message publishing".to_string(),
            ));
        }

        Ok(())
    }

    /// Send a text message to the LiveKit room in the specified JSON format
    ///
    /// This is a convenience method that creates a JSON object with the format:
    /// `{"message": "your text message here", "role": "custom role"}` and publishes it to the specified topic.
    ///
    /// # Arguments
    /// * `message` - The text message to send
    /// * `role` - The custom defined string role
    /// * `topic` - Optional topic/channel (defaults to "messages" if None)
    ///
    /// # Returns
    /// * `Ok(())` - Message sent successfully
    /// * `Err(AppError)` - Failed to send message
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
    /// // Send a message to the default "messages" topic
    /// client.send_message("Hello, everyone!", "user", None).await?;
    ///
    /// // Send a message to a specific topic
    /// client.send_message("TTS synthesis complete", "system", Some("tts-status")).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_message(
        &mut self,
        message: &str,
        role: &str,
        topic: Option<&str>,
    ) -> Result<(), AppError> {
        let topic = topic.unwrap_or("messages");
        let json_message = json!({
            "message": message,
            "role": role
        });

        info!(
            "Sending message to topic '{}': {} (role: {})",
            topic, message, role
        );
        self.send_data_message(topic, json_message).await
    }

    /// Clear all pending buffered audio from the published track
    ///
    /// This method clears the local audio buffer that hasn't been sent to WebRTC yet.
    /// 
    /// Important limitations:
    /// - Audio already sent to WebRTC (every 10ms) cannot be recalled
    /// - There may be 10-50ms of audio that continues playing
    /// - This is the same limitation that Python's clear_queue() has
    ///
    /// For true audio interruption in voice agents, consider:
    /// - Using shorter audio chunks for TTS
    /// - Implementing streaming TTS that can be stopped mid-stream
    /// - Managing interruptions at the application level
    ///
    /// # Returns
    /// * `Ok(())` - Audio buffer cleared successfully
    /// * `Err(AppError)` - Failed to clear audio buffer
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
    /// // Send some TTS audio
    /// let tts_audio_data = vec![0u8; 1024];
    /// client.send_tts_audio(tts_audio_data).await?;
    ///
    /// // Clear any pending buffered audio
    /// client.clear_audio().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn clear_audio(&mut self) -> Result<(), AppError> {
        debug!("Clearing pending buffered audio from LiveKit track");

        if !*self.is_connected.lock().await {
            return Err(AppError::InternalServerError(
                "Not connected to LiveKit room".to_string(),
            ));
        }

        // Clear the local buffer in the NativeAudioSource
        // This is equivalent to Python SDK's clear_queue() method
        // It only clears audio that hasn't been sent to WebRTC yet
        if let Some(audio_source) = &self.audio_source {
            audio_source.clear_buffer();
            debug!("Cleared local audio buffer");
        } else {
            warn!("No audio source available - nothing to clear");
        }

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

                            let sample_rate = config.sample_rate;

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
                                    match reduce_noise_async(audio_data.into(), sample_rate).await {
                                        Ok(audio_data) => callback_clone(audio_data),
                                        Err(e) => {
                                            error!("Error reducing noise: {:?}", e)
                                        }
                                    };
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{Duration, timeout};

    fn create_test_config() -> LiveKitConfig {
        LiveKitConfig {
            url: "wss://test-server.com".to_string(),
            token: "mock-jwt-token".to_string(),
            sample_rate: 24000,
            channels: 1,
        }
    }

    #[tokio::test]
    async fn test_livekit_client_creation() {
        let config = create_test_config();
        let client = LiveKitClient::new(config);

        assert!(!client.is_connected().await);
        assert!(client.audio_source.is_none());
        assert!(client.local_audio_track.is_none());
        assert!(client.local_track_publication.is_none());
    }

    #[tokio::test]
    async fn test_livekit_client_clear_audio_not_connected() {
        let config = create_test_config();
        let mut client = LiveKitClient::new(config);

        // Should fail when not connected
        let result = client.clear_audio().await;
        assert!(
            result.is_err(),
            "clear_audio should fail when not connected"
        );

        if let Err(AppError::InternalServerError(msg)) = result {
            assert!(
                msg.contains("Not connected"),
                "Error message should mention not connected"
            );
        } else {
            panic!("Expected InternalServerError about not being connected");
        }
    }

    #[tokio::test]
    async fn test_livekit_client_clear_audio_no_audio_source() {
        let config = create_test_config();
        let mut client = LiveKitClient::new(config);

        // Manually set connected state for testing
        *client.is_connected.lock().await = true;

        // Should succeed when connected but no audio source
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

        // Test audio callback
        client.set_audio_callback(|_data| {
            // Mock callback
        });
        assert!(client.audio_callback.is_some());

        // Test data callback
        client.set_data_callback(|_data| {
            // Mock callback
        });
        assert!(client.data_callback.is_some());

        // Test participant disconnect callback
        client.set_participant_disconnect_callback(|_event| {
            // Mock callback
        });
        assert!(client.participant_disconnect_callback.is_some());
    }

    #[tokio::test]
    async fn test_livekit_client_audio_frame_conversion() {
        let config = create_test_config();
        let client = LiveKitClient::new(config);

        // Test audio frame conversion with valid data
        let audio_data = vec![0u8, 1u8, 2u8, 3u8]; // 2 samples in little-endian
        let result = client.convert_tts_to_audio_frame(audio_data);

        assert!(
            result.is_ok(),
            "Audio frame conversion should succeed with valid data"
        );

        let audio_frame = result.unwrap();
        assert_eq!(audio_frame.sample_rate, 24000);
        assert_eq!(audio_frame.num_channels, 1);
        assert_eq!(audio_frame.samples_per_channel, 2);
    }

    #[tokio::test]
    async fn test_livekit_client_audio_frame_conversion_invalid_data() {
        let config = create_test_config();
        let client = LiveKitClient::new(config);

        // Test audio frame conversion with invalid data (odd number of bytes)
        let audio_data = vec![0u8, 1u8, 2u8]; // 3 bytes - not divisible by 2
        let result = client.convert_tts_to_audio_frame(audio_data);

        assert!(
            result.is_err(),
            "Audio frame conversion should fail with invalid data"
        );

        if let Err(AppError::InternalServerError(msg)) = result {
            assert!(
                msg.contains("even"),
                "Error message should mention even length requirement"
            );
        }
    }

    #[tokio::test]
    async fn test_livekit_client_clear_audio_timing() {
        let config = create_test_config();
        let mut client = LiveKitClient::new(config);

        // Manually set connected state for testing
        *client.is_connected.lock().await = true;

        // Test that clear_audio completes within reasonable time
        let result = timeout(Duration::from_millis(100), client.clear_audio()).await;

        assert!(result.is_ok(), "clear_audio should complete within 100ms");
        assert!(result.unwrap().is_ok(), "clear_audio should succeed");
    }

    #[tokio::test]
    async fn test_livekit_client_send_tts_audio_not_connected() {
        let config = create_test_config();
        let mut client = LiveKitClient::new(config);

        let audio_data = vec![0u8; 1024];
        let result = client.send_tts_audio(audio_data).await;

        assert!(
            result.is_err(),
            "send_tts_audio should fail when not connected"
        );
    }

    #[tokio::test]
    async fn test_livekit_client_send_tts_audio_no_source() {
        let config = create_test_config();
        let mut client = LiveKitClient::new(config);

        // Manually set connected state for testing
        *client.is_connected.lock().await = true;

        let audio_data = vec![0u8; 1024];
        let result = client.send_tts_audio(audio_data).await;

        // Should succeed but warn about no audio source
        assert!(
            result.is_ok(),
            "send_tts_audio should succeed when no audio source (with warning)"
        );
    }

    #[tokio::test]
    async fn test_livekit_client_multiple_clear_audio_calls() {
        let config = create_test_config();
        let mut client = LiveKitClient::new(config);

        // Manually set connected state for testing
        *client.is_connected.lock().await = true;

        // Test multiple clear_audio calls in succession
        for i in 0..3 {
            let result = client.clear_audio().await;
            assert!(result.is_ok(), "clear_audio call {} should succeed", i + 1);
        }
    }

    #[tokio::test]
    async fn test_livekit_client_clear_audio_state_consistency() {
        let config = create_test_config();
        let mut client = LiveKitClient::new(config);

        // Manually set connected state for testing
        *client.is_connected.lock().await = true;

        // Verify initial state
        assert!(client.is_connected().await);
        assert!(client.audio_source.is_none());

        // Clear audio
        let result = client.clear_audio().await;
        assert!(result.is_ok());

        // Verify state is still consistent
        assert!(client.is_connected().await);
    }

    #[tokio::test]
    async fn test_livekit_client_data_message_serialization() {
        use serde_json::json;

        let config = create_test_config();
        let mut client = LiveKitClient::new(config);

        // Manually set connected state for testing
        *client.is_connected.lock().await = true;

        let test_data = json!({
            "type": "test",
            "message": "hello world"
        });

        let result = client.send_data_message("test-topic", test_data).await;

        // This will fail because no room is set up, but we're testing the serialization path
        assert!(
            result.is_err(),
            "send_data_message should fail without room setup"
        );
    }

    #[tokio::test]
    async fn test_livekit_client_disconnect_cleanup() {
        let config = create_test_config();
        let mut client = LiveKitClient::new(config);

        // Manually set some state for testing
        *client.is_connected.lock().await = true;

        let result = client.disconnect().await;
        assert!(result.is_ok(), "Disconnect should succeed");

        // Verify cleanup
        assert!(!client.is_connected().await);
        assert!(client.audio_source.is_none());
        assert!(client.local_audio_track.is_none());
        assert!(client.local_track_publication.is_none());
        assert!(client.room.is_none());
        assert!(client.room_events.is_none());
    }

    #[tokio::test]
    async fn test_livekit_client_clear_audio_integration_pattern() {
        // This test mimics the integration pattern used in the WebSocket handler
        let config = create_test_config();
        let mut client = LiveKitClient::new(config);

        // Manually set connected state for testing
        *client.is_connected.lock().await = true;

        // Simulate the pattern: send some audio, then clear
        let audio_data = vec![0u8; 1024];
        let _send_result = client.send_tts_audio(audio_data).await;

        // Clear the audio buffer
        let clear_result = client.clear_audio().await;
        assert!(
            clear_result.is_ok(),
            "clear_audio should succeed in integration pattern"
        );

        // Should be able to send more audio after clearing
        let audio_data2 = vec![1u8; 512];
        let _send_result2 = client.send_tts_audio(audio_data2).await;
    }
}
