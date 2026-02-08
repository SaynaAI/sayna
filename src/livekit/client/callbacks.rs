//! Callback registration utilities for `LiveKitClient`.
//!
//! These helpers allow the host application to register handlers for audio,
//! data, and participant lifecycle events without exposing internal client
//! state.

use std::sync::Arc;

use super::{
    DataMessage, LiveKitClient, ParticipantConnectEvent, ParticipantDisconnectEvent,
    TrackSubscribedEvent,
};

impl LiveKitClient {
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
    /// #     room_name: "test-room".to_string(),
    /// #     sample_rate: 24000,
    /// #     channels: 1,
    /// #     enable_noise_filter: cfg!(feature = "noise-filter"),
    /// #     listen_participants: vec![],
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
    /// #     room_name: "test-room".to_string(),
    /// #     sample_rate: 24000,
    /// #     channels: 1,
    /// #     enable_noise_filter: cfg!(feature = "noise-filter"),
    /// #     listen_participants: vec![],
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

    /// Register a callback to handle participant disconnection events.
    ///
    /// The callback receives information about the disconnected participant including
    /// their identity, display name (if available), and the timestamp of disconnection.
    pub fn set_participant_disconnect_callback<F>(&mut self, callback: F)
    where
        F: Fn(ParticipantDisconnectEvent) + Send + Sync + 'static,
    {
        self.participant_disconnect_callback = Some(Arc::new(callback));
    }

    /// Register a callback to handle participant connection events.
    ///
    /// The callback receives information about the connected participant including
    /// their identity, display name (if available), and the timestamp of connection.
    pub fn set_participant_connect_callback<F>(&mut self, callback: F)
    where
        F: Fn(ParticipantConnectEvent) + Send + Sync + 'static,
    {
        self.participant_connect_callback = Some(Arc::new(callback));
    }

    /// Register a callback to handle track subscription events.
    ///
    /// The callback receives information about the subscribed track including
    /// the participant identity, track kind (audio/video), and track SID.
    pub fn set_track_subscribed_callback<F>(&mut self, callback: F)
    where
        F: Fn(TrackSubscribedEvent) + Send + Sync + 'static,
    {
        self.track_subscribed_callback = Some(Arc::new(callback));
    }
}
