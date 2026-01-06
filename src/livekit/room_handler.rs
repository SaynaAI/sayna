//! # LiveKit Room Handler
//!
//! This module provides functionality for managing LiveKit rooms and generating access tokens.
//! It mirrors the Python LiveKitTokenHandler functionality but implemented in Rust using the
//! livekit-api crate.
//!
//! ## Features
//!
//! - **Room Creation**: Create LiveKit rooms with configurable parameters
//! - **Token Generation**: Generate JWT tokens for users and agents with specific permissions
//! - **Permission Management**: Configure video grants for room access, publishing, and recording
//!
//! ## Usage Example
//!
//! ```rust,no_run
//! use sayna::livekit::room_handler::LiveKitRoomHandler;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let handler = LiveKitRoomHandler::new(
//!         "http://localhost:7880".to_string(),
//!         "api_key".to_string(),
//!         "api_secret".to_string(),
//!         None,
//!     )?;
//!
//!     // Create a room
//!     handler.create_room("my-room").await?;
//!
//!     // Generate user token
//!     let user_token = handler.user_token("my-room", "user-123", "Alice")?;
//!
//!     // Generate agent token with admin privileges
//!     let agent_token = handler.agent_token("my-room", "sayna-ai", "Sayna AI")?;
//!
//!     Ok(())
//! }
//! ```

use livekit_api::access_token::{AccessToken, SIPGrants, VideoGrants};
use livekit_api::services::egress::{EgressClient, EgressOutput, RoomCompositeOptions};
use livekit_api::services::room::{CreateRoomOptions, RoomClient};
use livekit_protocol as proto;

use super::types::LiveKitError;

/// Configuration for S3 recording uploads
#[derive(Debug, Clone)]
pub struct RecordingConfig {
    /// S3 bucket for audio recordings
    pub bucket: String,
    /// AWS S3 region
    pub region: String,
    /// AWS S3 endpoint
    pub endpoint: String,
    /// AWS S3 access key
    pub access_key: String,
    /// AWS S3 secret key
    pub secret_key: String,
    /// S3 path prefix for recordings.
    /// Combined with stream_id to construct full path: `{prefix}/{stream_id}/audio.ogg`
    pub prefix: String,
}

// Build the full recording file path from prefix and stream_id.
fn build_recording_filepath(prefix: &str, stream_id: &str) -> String {
    if prefix.is_empty() {
        format!("{stream_id}/audio.ogg")
    } else {
        let prefix = prefix.trim_end_matches('/');
        format!("{prefix}/{stream_id}/audio.ogg")
    }
}

/// Handler for LiveKit room management and token generation
///
/// This struct provides methods to create LiveKit rooms and generate JWT tokens
/// for participants with different permission levels.
pub struct LiveKitRoomHandler {
    /// LiveKit server URL (e.g., "http://localhost:7880")
    url: String,
    /// API key for authentication
    api_key: String,
    /// API secret for JWT signing
    api_secret: String,
    /// RoomClient for room operations
    room_client: RoomClient,
    /// EgressClient for recording operations
    egress_client: EgressClient,
    /// Optional recording configuration
    recording_config: Option<RecordingConfig>,
}

impl LiveKitRoomHandler {
    /// Create a new LiveKitRoomHandler
    ///
    /// # Arguments
    /// * `url` - LiveKit server URL
    /// * `api_key` - LiveKit API key
    /// * `api_secret` - LiveKit API secret
    /// * `recording_config` - Optional recording configuration for S3 uploads
    ///
    /// # Returns
    /// * `Result<Self, LiveKitError>` - New handler instance or error
    ///
    /// # Example
    /// ```rust,no_run
    /// use sayna::livekit::room_handler::LiveKitRoomHandler;
    ///
    /// let handler = LiveKitRoomHandler::new(
    ///     "http://localhost:7880".to_string(),
    ///     "api_key".to_string(),
    ///     "api_secret".to_string(),
    ///     None,
    /// )?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn new(
        url: String,
        api_key: String,
        api_secret: String,
        recording_config: Option<RecordingConfig>,
    ) -> Result<Self, LiveKitError> {
        let room_client = RoomClient::with_api_key(&url, &api_key, &api_secret);
        let egress_client = EgressClient::with_api_key(&url, &api_key, &api_secret);

        Ok(Self {
            url,
            api_key,
            api_secret,
            room_client,
            egress_client,
            recording_config,
        })
    }

    /// Create video grants permissions for a room
    ///
    /// # Arguments
    /// * `room_name` - Name of the LiveKit room
    /// * `room_admin` - Whether to grant admin privileges
    ///
    /// # Returns
    /// * `VideoGrants` - Configured video grants with permissions
    ///
    /// Permissions granted:
    /// - room_join: Join the room
    /// - room_admin: Admin privileges (if requested)
    /// - room_record: Record the room
    /// - room_list: List rooms
    /// - can_publish: Publish audio/video tracks
    /// - can_subscribe: Subscribe to other participants' tracks
    /// - can_publish_data: Send data messages
    /// - can_update_own_metadata: Update own participant metadata
    /// - room_create: Create new rooms
    fn token_permissions(&self, room_name: &str, room_admin: bool) -> VideoGrants {
        VideoGrants {
            room: room_name.to_string(),
            room_join: true,
            room_admin,
            room_record: true,
            room_list: true,
            can_publish: true,
            can_subscribe: true,
            can_publish_data: true,
            can_update_own_metadata: true,
            room_create: true,
            ..Default::default()
        }
    }

    /// Generate a JWT token for a regular user
    ///
    /// # Arguments
    /// * `room_name` - Name of the LiveKit room
    /// * `identity` - Unique identifier for the user participant
    /// * `name` - Display name for the user participant
    ///
    /// # Returns
    /// * `Result<String, LiveKitError>` - JWT token string for user authorization
    ///
    /// # Example
    /// ```rust,no_run
    /// use sayna::livekit::room_handler::LiveKitRoomHandler;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let handler = LiveKitRoomHandler::new(
    ///     "http://localhost:7880".to_string(),
    ///     "api_key".to_string(),
    ///     "api_secret".to_string(),
    ///     None,
    /// )?;
    ///
    /// let token = handler.user_token("my-room", "user-123", "Alice")?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn user_token(
        &self,
        room_name: &str,
        identity: &str,
        name: &str,
    ) -> Result<String, LiveKitError> {
        let token = AccessToken::with_api_key(&self.api_key, &self.api_secret)
            .with_identity(identity)
            .with_name(name)
            .with_grants(self.token_permissions(room_name, false))
            .to_jwt()
            .map_err(|e| {
                LiveKitError::ConnectionFailed(format!("Failed to generate user token: {e}"))
            })?;

        Ok(token)
    }

    /// Generate a JWT token for an agent with admin privileges
    ///
    /// # Arguments
    /// * `room_name` - Name of the LiveKit room
    /// * `identity` - Unique identifier for the agent participant
    /// * `name` - Display name for the agent participant
    ///
    /// # Returns
    /// * `Result<String, LiveKitError>` - JWT token string for agent authorization
    ///
    /// # Example
    /// ```rust,no_run
    /// use sayna::livekit::room_handler::LiveKitRoomHandler;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let handler = LiveKitRoomHandler::new(
    ///     "http://localhost:7880".to_string(),
    ///     "api_key".to_string(),
    ///     "api_secret".to_string(),
    ///     None,
    /// )?;
    ///
    /// let token = handler.agent_token("my-room", "sayna-ai", "Sayna AI")?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn agent_token(
        &self,
        room_name: &str,
        identity: &str,
        name: &str,
    ) -> Result<String, LiveKitError> {
        let token = AccessToken::with_api_key(&self.api_key, &self.api_secret)
            .with_identity(identity)
            .with_name(name)
            .with_grants(self.token_permissions(room_name, true))
            .to_jwt()
            .map_err(|e| {
                LiveKitError::ConnectionFailed(format!("Failed to generate agent token: {e}"))
            })?;

        Ok(token)
    }

    /// Generate a JWT token for an agent with admin privileges and SIP admin grants
    ///
    /// This is used for the websocket LiveKit connection where the server needs
    /// elevated SIP permissions to manage transfers, while keeping other tokens
    /// non-admin.
    pub fn agent_token_with_sip_admin(
        &self,
        room_name: &str,
        identity: &str,
        name: &str,
    ) -> Result<String, LiveKitError> {
        let token = AccessToken::with_api_key(&self.api_key, &self.api_secret)
            .with_identity(identity)
            .with_name(name)
            .with_grants(self.token_permissions(room_name, true))
            .with_sip_grants(SIPGrants {
                admin: true,
                ..Default::default()
            })
            .to_jwt()
            .map_err(|e| {
                LiveKitError::ConnectionFailed(format!(
                    "Failed to generate agent token with SIP grants: {e}"
                ))
            })?;

        Ok(token)
    }

    /// Create a new LiveKit room
    ///
    /// # Arguments
    /// * `room_name` - Name of the LiveKit room to create
    ///
    /// # Returns
    /// * `Result<(), LiveKitError>` - Success or error
    ///
    /// # Example
    /// ```rust,no_run
    /// use sayna::livekit::room_handler::LiveKitRoomHandler;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let handler = LiveKitRoomHandler::new(
    ///     "http://localhost:7880".to_string(),
    ///     "api_key".to_string(),
    ///     "api_secret".to_string(),
    ///     None,
    /// )?;
    ///
    /// handler.create_room("my-room").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn create_room(&self, room_name: &str) -> Result<(), LiveKitError> {
        let options = CreateRoomOptions {
            max_participants: 3,
            ..Default::default()
        };

        self.room_client
            .create_room(room_name, options)
            .await
            .map_err(|e| LiveKitError::ConnectionFailed(format!("Failed to create room: {e}")))?;

        Ok(())
    }

    /// Get the LiveKit server URL
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Get the API key
    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    /// Delete a LiveKit room
    ///
    /// # Arguments
    /// * `room_name` - Name of the LiveKit room to delete
    ///
    /// # Returns
    /// * `Result<(), LiveKitError>` - Success or error
    ///
    /// # Example
    /// ```rust,no_run
    /// use sayna::livekit::room_handler::LiveKitRoomHandler;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let handler = LiveKitRoomHandler::new(
    ///     "http://localhost:7880".to_string(),
    ///     "api_key".to_string(),
    ///     "api_secret".to_string(),
    ///     None,
    /// )?;
    ///
    /// handler.delete_room("my-room").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn delete_room(&self, room_name: &str) -> Result<(), LiveKitError> {
        self.room_client
            .delete_room(room_name)
            .await
            .map_err(|e| LiveKitError::ConnectionFailed(format!("Failed to delete room: {e}")))?;

        Ok(())
    }

    /// Setup room recording with S3 upload
    ///
    /// # Arguments
    /// * `room_name` - Name of the LiveKit room to record
    /// * `stream_id` - Unique identifier for the recording stream
    ///
    /// # Returns
    /// * `Result<String, LiveKitError>` - Egress ID for the started recording or error
    ///
    /// # Path Construction
    ///
    /// The recording path is constructed as: `{prefix}/{stream_id}/audio.ogg`
    ///
    /// Examples:
    /// - Prefix: `recordings/prod`, Stream ID: `abc-123` -> `recordings/prod/abc-123/audio.ogg`
    /// - Prefix: `""` (empty), Stream ID: `abc-123` -> `abc-123/audio.ogg`
    /// - Prefix: `data/` (trailing slash), Stream ID: `xyz` -> `data/xyz/audio.ogg`
    ///
    /// # Example
    /// ```rust,no_run
    /// use sayna::livekit::room_handler::LiveKitRoomHandler;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// use sayna::livekit::room_handler::RecordingConfig;
    /// let handler = LiveKitRoomHandler::new(
    ///     "http://localhost:7880".to_string(),
    ///     "api_key".to_string(),
    ///     "api_secret".to_string(),
    ///     Some(RecordingConfig {
    ///         bucket: "my-bucket".to_string(),
    ///         region: "us-east-1".to_string(),
    ///         endpoint: "https://s3.amazonaws.com".to_string(),
    ///         access_key: "access_key".to_string(),
    ///         secret_key: "secret_key".to_string(),
    ///         prefix: "recordings/prod".to_string(),
    ///     }),
    /// )?;
    ///
    /// let egress_id = handler
    ///     .setup_room_recording(
    ///         "my-room",
    ///         "550e8400-e29b-41d4-a716-446655440000",
    ///     )
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn setup_room_recording(
        &self,
        room_name: &str,
        stream_id: &str,
    ) -> Result<String, LiveKitError> {
        // Validate that recording configuration is present
        let config = self.recording_config.as_ref().ok_or_else(|| {
            LiveKitError::ConnectionFailed("Recording configuration not provided".to_string())
        })?;

        // Create S3 upload configuration
        let s3_upload = proto::S3Upload {
            bucket: config.bucket.clone(),
            region: config.region.clone(),
            endpoint: config.endpoint.clone(),
            access_key: config.access_key.clone(),
            secret: config.secret_key.clone(),
            force_path_style: true,
            ..Default::default()
        };

        // Construct recording path: {prefix}/{stream_id}/audio.ogg
        let filepath = build_recording_filepath(&config.prefix, stream_id);

        // Create encoded file output with S3 upload
        let file_output = proto::EncodedFileOutput {
            file_type: proto::EncodedFileType::Ogg as i32,
            filepath,
            disable_manifest: false,
            output: Some(proto::encoded_file_output::Output::S3(s3_upload)),
        };

        // Configure room composite options for audio-only recording
        let options = RoomCompositeOptions {
            audio_only: true,
            ..Default::default()
        };

        // Start the room composite egress
        let egress_info = self
            .egress_client
            .start_room_composite_egress(room_name, vec![EgressOutput::File(file_output)], options)
            .await
            .map_err(|e| {
                LiveKitError::ConnectionFailed(format!("Failed to start room recording: {e}"))
            })?;

        Ok(egress_info.egress_id)
    }

    /// Stop room recording
    ///
    /// # Arguments
    /// * `egress_id` - The egress ID returned from setup_room_recording
    ///
    /// # Returns
    /// * `Result<(), LiveKitError>` - Success or error
    ///
    /// # Example
    /// ```rust,no_run
    /// use sayna::livekit::room_handler::LiveKitRoomHandler;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let handler = LiveKitRoomHandler::new(
    ///     "http://localhost:7880".to_string(),
    ///     "api_key".to_string(),
    ///     "api_secret".to_string(),
    ///     None,
    /// )?;
    ///
    /// handler.stop_room_recording("egress-id-123").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn stop_room_recording(&self, egress_id: &str) -> Result<(), LiveKitError> {
        self.egress_client
            .stop_egress(egress_id)
            .await
            .map_err(|e| {
                LiveKitError::ConnectionFailed(format!("Failed to stop room recording: {e}"))
            })?;

        Ok(())
    }

    /// List participants in a LiveKit room
    ///
    /// # Arguments
    /// * `room_name` - Name of the LiveKit room
    ///
    /// # Returns
    /// * `Result<Vec<proto::ParticipantInfo>, LiveKitError>` - List of participants or error
    ///
    /// # Example
    /// ```rust,no_run
    /// use sayna::livekit::room_handler::LiveKitRoomHandler;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let handler = LiveKitRoomHandler::new(
    ///     "http://localhost:7880".to_string(),
    ///     "api_key".to_string(),
    ///     "api_secret".to_string(),
    ///     None,
    /// )?;
    ///
    /// let participants = handler.list_participants("my-room").await?;
    /// for p in participants {
    ///     println!("Participant: {} ({})", p.name, p.identity);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list_participants(
        &self,
        room_name: &str,
    ) -> Result<Vec<proto::ParticipantInfo>, LiveKitError> {
        self.room_client
            .list_participants(room_name)
            .await
            .map_err(|e| {
                LiveKitError::ConnectionFailed(format!("Failed to list participants: {e}"))
            })
    }

    /// List all LiveKit rooms, optionally filtered by a name prefix
    ///
    /// # Arguments
    /// * `prefix` - Optional prefix to filter room names. If None, returns all rooms.
    ///
    /// # Returns
    /// * `Result<Vec<proto::Room>, LiveKitError>` - List of rooms or error
    ///
    /// # Example
    /// ```rust,no_run
    /// use sayna::livekit::room_handler::LiveKitRoomHandler;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let handler = LiveKitRoomHandler::new(
    ///     "http://localhost:7880".to_string(),
    ///     "api_key".to_string(),
    ///     "api_secret".to_string(),
    ///     None,
    /// )?;
    ///
    /// // List all rooms for tenant "project1"
    /// let rooms = handler.list_rooms(Some("project1_")).await?;
    /// for room in rooms {
    ///     println!("Room: {}", room.name);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list_rooms(&self, prefix: Option<&str>) -> Result<Vec<proto::Room>, LiveKitError> {
        // Fetch all rooms from LiveKit
        let all_rooms =
            self.room_client.list_rooms(vec![]).await.map_err(|e| {
                LiveKitError::ConnectionFailed(format!("Failed to list rooms: {e}"))
            })?;

        // Filter by prefix if provided
        let rooms = match prefix {
            Some(p) if !p.is_empty() => all_rooms
                .into_iter()
                .filter(|r| r.name.starts_with(p))
                .collect(),
            _ => all_rooms,
        };

        Ok(rooms)
    }

    /// Remove a participant from a LiveKit room
    ///
    /// This forcibly disconnects the participant from the room. Note that this
    /// does not invalidate the participant's token - they can rejoin if they
    /// still have a valid token.
    ///
    /// # Arguments
    /// * `room_name` - Name of the LiveKit room
    /// * `identity` - Identity of the participant to remove
    ///
    /// # Returns
    /// * `Result<(), LiveKitError>` - Success or error
    ///
    /// # Example
    /// ```rust,no_run
    /// use sayna::livekit::room_handler::LiveKitRoomHandler;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let handler = LiveKitRoomHandler::new(
    ///     "http://localhost:7880".to_string(),
    ///     "api_key".to_string(),
    ///     "api_secret".to_string(),
    ///     None,
    /// )?;
    ///
    /// // Remove participant from room
    /// handler.remove_participant("my-room", "user-123").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn remove_participant(
        &self,
        room_name: &str,
        identity: &str,
    ) -> Result<(), LiveKitError> {
        self.room_client
            .remove_participant(room_name, identity)
            .await
            .map_err(|e| {
                LiveKitError::ConnectionFailed(format!("Failed to remove participant: {e}"))
            })
    }

    /// Get detailed information about a specific LiveKit room including participants
    ///
    /// # Arguments
    /// * `room_name` - Name of the LiveKit room
    ///
    /// # Returns
    /// * `Result<(proto::Room, Vec<proto::ParticipantInfo>), LiveKitError>` - Room info and participants or error
    ///
    /// # Example
    /// ```rust,no_run
    /// use sayna::livekit::room_handler::LiveKitRoomHandler;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let handler = LiveKitRoomHandler::new(
    ///     "http://localhost:7880".to_string(),
    ///     "api_key".to_string(),
    ///     "api_secret".to_string(),
    ///     None,
    /// )?;
    ///
    /// // Get room details with participants
    /// let (room, participants) = handler.get_room_details("my-room").await?;
    /// println!("Room: {}, Participants: {}", room.name, participants.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_room_details(
        &self,
        room_name: &str,
    ) -> Result<(proto::Room, Vec<proto::ParticipantInfo>), LiveKitError> {
        // Get room info by querying with specific room name
        let rooms = self
            .room_client
            .list_rooms(vec![room_name.to_string()])
            .await
            .map_err(|e| LiveKitError::ConnectionFailed(format!("Failed to get room: {e}")))?;

        let room = rooms.into_iter().next().ok_or_else(|| {
            LiveKitError::ConnectionFailed(format!("Room '{}' not found", room_name))
        })?;

        // Get participants in the room
        let participants = self
            .room_client
            .list_participants(room_name)
            .await
            .map_err(|e| {
                LiveKitError::ConnectionFailed(format!("Failed to list participants: {e}"))
            })?;

        Ok((room, participants))
    }

    /// Mute or unmute a participant's published track
    ///
    /// # Arguments
    /// * `room_name` - Name of the LiveKit room
    /// * `identity` - Identity of the participant
    /// * `track_sid` - Session ID of the track to mute/unmute
    /// * `muted` - True to mute, false to unmute
    ///
    /// # Returns
    /// * `Result<proto::TrackInfo, LiveKitError>` - Updated track info or error
    pub async fn mute_participant_track(
        &self,
        room_name: &str,
        identity: &str,
        track_sid: &str,
        muted: bool,
    ) -> Result<proto::TrackInfo, LiveKitError> {
        self.room_client
            .mute_published_track(room_name, identity, track_sid, muted)
            .await
            .map_err(|e| LiveKitError::ConnectionFailed(format!("Failed to mute track: {e}")))
    }

    /// Update room metadata for a specified room
    ///
    /// This is a low-level method that directly sets the room metadata. For setting
    /// the `auth_id` field specifically, prefer using [`ensure_room_auth_id`] which
    /// handles conflict detection and preserves existing metadata.
    ///
    /// # Arguments
    /// * `room_name` - Name of the LiveKit room
    /// * `metadata` - New metadata string (typically JSON)
    ///
    /// # Returns
    /// * `Result<(), LiveKitError>` - Success or error
    ///
    /// # Example
    /// ```rust,no_run
    /// use sayna::livekit::room_handler::LiveKitRoomHandler;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let handler = LiveKitRoomHandler::new(
    ///     "http://localhost:7880".to_string(),
    ///     "api_key".to_string(),
    ///     "api_secret".to_string(),
    ///     None,
    /// )?;
    ///
    /// handler.update_room_metadata("my-room", r#"{"auth_id": "tenant-123"}"#).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn update_room_metadata(
        &self,
        room_name: &str,
        metadata: &str,
    ) -> Result<(), LiveKitError> {
        self.room_client
            .update_room_metadata(room_name, metadata)
            .await
            .map_err(|e| {
                LiveKitError::ConnectionFailed(format!("Failed to update room metadata: {e}"))
            })?;

        Ok(())
    }

    /// Ensure a room has the specified `auth_id` in its metadata
    ///
    /// This method safely associates a room with a tenant identifier (`auth_id`) by:
    /// - Reading the current room metadata
    /// - Merging the `auth_id` into existing metadata (preserving other keys)
    /// - Only updating if the metadata actually changed
    ///
    /// # Security
    ///
    /// This method prevents cross-tenant hijacking: if the room already has a
    /// different `auth_id`, it returns a `MetadataConflict` error rather than
    /// overwriting the existing value.
    ///
    /// # Arguments
    /// * `room_name` - Name of the LiveKit room
    /// * `auth_id` - The tenant identifier to associate with the room
    ///
    /// # Returns
    /// * `Ok(())` - Room metadata now contains the auth_id
    /// * `Err(MetadataConflict)` - Room has a different auth_id (cross-tenant attempt)
    /// * `Err(RoomNotFound)` - Room does not exist
    /// * `Err(ConnectionFailed)` - Network or API error
    ///
    /// # Example
    /// ```rust,no_run
    /// use sayna::livekit::room_handler::LiveKitRoomHandler;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let handler = LiveKitRoomHandler::new(
    ///     "http://localhost:7880".to_string(),
    ///     "api_key".to_string(),
    ///     "api_secret".to_string(),
    ///     None,
    /// )?;
    ///
    /// // Associate room with tenant
    /// handler.ensure_room_auth_id("my-room", "tenant-123").await?;
    ///
    /// // Calling again with same auth_id is idempotent
    /// handler.ensure_room_auth_id("my-room", "tenant-123").await?;
    ///
    /// // Different auth_id returns error
    /// let result = handler.ensure_room_auth_id("my-room", "tenant-456").await;
    /// assert!(result.is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn ensure_room_auth_id(
        &self,
        room_name: &str,
        auth_id: &str,
    ) -> Result<(), LiveKitError> {
        use super::metadata::{MetadataError, get_auth_id, merge_auth_id};

        // Get current room details
        let rooms = self
            .room_client
            .list_rooms(vec![room_name.to_string()])
            .await
            .map_err(|e| LiveKitError::ConnectionFailed(format!("Failed to get room: {e}")))?;

        let room = rooms
            .into_iter()
            .next()
            .ok_or_else(|| LiveKitError::RoomNotFound(room_name.to_string()))?;

        let current_metadata = &room.metadata;

        // Check if auth_id is already set correctly (optimization to avoid unnecessary update)
        if let Some(existing_id) = get_auth_id(current_metadata)
            && existing_id == auth_id
        {
            // Already set correctly, nothing to do
            return Ok(());
        }

        // Merge auth_id into metadata
        let new_metadata = merge_auth_id(current_metadata, auth_id).map_err(|e| match e {
            MetadataError::AuthIdConflict {
                existing,
                attempted,
            } => LiveKitError::MetadataConflict {
                existing,
                attempted,
            },
            MetadataError::InvalidJson(msg) => {
                LiveKitError::ConnectionFailed(format!("Invalid room metadata JSON: {msg}"))
            }
            MetadataError::NotAnObject(msg) => {
                LiveKitError::ConnectionFailed(format!("Room metadata is not a JSON object: {msg}"))
            }
        })?;

        // Update room metadata
        self.update_room_metadata(room_name, &new_metadata).await
    }

    /// Ensure a room exists and has the correct `auth_id` in its metadata.
    ///
    /// This method implements a create-if-not-exists pattern with proper race condition handling:
    /// 1. Try to ensure the room has the correct auth_id
    /// 2. If the room doesn't exist, create it
    /// 3. Retry ensuring auth_id after creation (handles race conditions)
    ///
    /// # Arguments
    /// * `room_name` - Name of the room to create/verify
    /// * `auth_id` - Tenant identifier to set in room metadata
    ///
    /// # Returns
    /// * `Ok(())` - Room exists (or was created) with correct auth_id
    /// * `Err(LiveKitError::MetadataConflict)` - Room exists with different auth_id
    /// * `Err(LiveKitError::RoomNotFound)` - Room could not be created (unexpected state)
    /// * `Err(LiveKitError::ConnectionFailed)` - Network or API error
    ///
    /// # Example
    /// ```rust,no_run
    /// use sayna::livekit::room_handler::LiveKitRoomHandler;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let handler = LiveKitRoomHandler::new(
    ///     "http://localhost:7880".to_string(),
    ///     "api_key".to_string(),
    ///     "api_secret".to_string(),
    ///     None,
    /// )?;
    ///
    /// // Creates room if missing and sets auth_id
    /// handler.ensure_room_with_auth_id("my-room", "tenant-123").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn ensure_room_with_auth_id(
        &self,
        room_name: &str,
        auth_id: &str,
    ) -> Result<(), LiveKitError> {
        use tracing::{debug, info, warn};

        // First attempt: check if room exists and has correct auth_id
        match self.ensure_room_auth_id(room_name, auth_id).await {
            Ok(()) => {
                debug!(room = %room_name, auth_id = %auth_id, "Room auth_id verified");
                return Ok(());
            }
            Err(LiveKitError::RoomNotFound(_)) => {
                debug!(room = %room_name, "Room not found, will create");
                // Fall through to create room
            }
            Err(e) => return Err(e),
        }

        // Room doesn't exist, try to create it
        match self.create_room(room_name).await {
            Ok(()) => {
                info!(room = %room_name, "Created room, now setting auth_id");
            }
            Err(e) => {
                // Creation failed - might be a race condition (room created by another process)
                // or a genuine error. Either way, retry ensure_room_auth_id to check.
                warn!(
                    room = %room_name,
                    error = %e,
                    "Room creation failed, retrying ensure_room_auth_id"
                );
            }
        }

        // Second attempt: room should now exist (either we created it or someone else did)
        self.ensure_room_auth_id(room_name, auth_id).await
    }

    /// Ensure a room exists (create if missing) without modifying metadata.
    ///
    /// This is used for unauthenticated mode where no tenant isolation is required.
    /// The room is created if it doesn't exist, but no metadata is set.
    ///
    /// # Arguments
    /// * `room_name` - Name of the room to create/verify
    ///
    /// # Returns
    /// * `Ok(())` - Room exists (or was created)
    /// * `Err(LiveKitError::ConnectionFailed)` - Network or API error
    ///
    /// # Example
    /// ```rust,no_run
    /// use sayna::livekit::room_handler::LiveKitRoomHandler;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let handler = LiveKitRoomHandler::new(
    ///     "http://localhost:7880".to_string(),
    ///     "api_key".to_string(),
    ///     "api_secret".to_string(),
    ///     None,
    /// )?;
    ///
    /// // Creates room if missing, does not set metadata
    /// handler.ensure_room_exists("my-room").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn ensure_room_exists(&self, room_name: &str) -> Result<(), LiveKitError> {
        use tracing::{debug, info, warn};

        // Try to check if room exists by querying it
        let rooms = self
            .room_client
            .list_rooms(vec![room_name.to_string()])
            .await
            .map_err(|e| LiveKitError::ConnectionFailed(format!("Failed to check room: {e}")))?;

        if rooms.iter().any(|r| r.name == room_name) {
            debug!(room = %room_name, "Room already exists");
            return Ok(());
        }

        // Room doesn't exist, try to create it
        match self.create_room(room_name).await {
            Ok(()) => {
                info!(room = %room_name, "Created room (no metadata)");
                Ok(())
            }
            Err(e) => {
                // Creation failed - might be race condition, re-check existence
                let rooms = self
                    .room_client
                    .list_rooms(vec![room_name.to_string()])
                    .await
                    .map_err(|e| {
                        LiveKitError::ConnectionFailed(format!("Failed to check room: {e}"))
                    })?;

                if rooms.iter().any(|r| r.name == room_name) {
                    debug!(room = %room_name, "Room exists after creation attempt (race condition)");
                    Ok(())
                } else {
                    warn!(room = %room_name, error = %e, "Room creation failed and room does not exist");
                    Err(e)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_room_handler_creation() {
        let handler = LiveKitRoomHandler::new(
            "http://localhost:7880".to_string(),
            "test_key".to_string(),
            "test_secret".to_string(),
            None,
        );

        assert!(handler.is_ok());
        let handler = handler.unwrap();
        assert_eq!(handler.url(), "http://localhost:7880");
        assert_eq!(handler.api_key(), "test_key");
    }

    #[test]
    fn test_token_permissions_user() {
        let handler = LiveKitRoomHandler::new(
            "http://localhost:7880".to_string(),
            "test_key".to_string(),
            "test_secret".to_string(),
            None,
        )
        .unwrap();

        let grants = handler.token_permissions("test-room", false);
        assert_eq!(grants.room, "test-room");
        assert!(grants.room_join);
        assert!(!grants.room_admin);
        assert!(grants.can_publish);
        assert!(grants.can_subscribe);
    }

    #[test]
    fn test_token_permissions_admin() {
        let handler = LiveKitRoomHandler::new(
            "http://localhost:7880".to_string(),
            "test_key".to_string(),
            "test_secret".to_string(),
            None,
        )
        .unwrap();

        let grants = handler.token_permissions("test-room", true);
        assert_eq!(grants.room, "test-room");
        assert!(grants.room_join);
        assert!(grants.room_admin);
        assert!(grants.can_publish);
        assert!(grants.can_subscribe);
    }

    #[test]
    fn test_user_token_generation() {
        let handler = LiveKitRoomHandler::new(
            "http://localhost:7880".to_string(),
            "test_key".to_string(),
            "test_secret".to_string(),
            None,
        )
        .unwrap();

        let result = handler.user_token("test-room", "test-user", "Test User");
        assert!(result.is_ok());
        let token = result.unwrap();
        assert!(!token.is_empty());
    }

    #[test]
    fn test_agent_token_generation() {
        let handler = LiveKitRoomHandler::new(
            "http://localhost:7880".to_string(),
            "test_key".to_string(),
            "test_secret".to_string(),
            None,
        )
        .unwrap();

        let result = handler.agent_token("test-room", "test-agent", "Test Agent");
        assert!(result.is_ok());
        let token = result.unwrap();
        assert!(!token.is_empty());
    }

    #[test]
    fn test_agent_token_with_sip_admin_generation() {
        let handler = LiveKitRoomHandler::new(
            "http://localhost:7880".to_string(),
            "test_key".to_string(),
            "test_secret".to_string(),
            None,
        )
        .unwrap();

        let result = handler.agent_token_with_sip_admin("test-room", "test-agent", "Test Agent");
        assert!(result.is_ok());
        let token = result.unwrap();
        assert!(!token.is_empty());
    }

    #[test]
    fn test_recording_configuration_validation() {
        // Handler without recording configuration
        let handler = LiveKitRoomHandler::new(
            "http://localhost:7880".to_string(),
            "test_key".to_string(),
            "test_secret".to_string(),
            None,
        )
        .unwrap();

        // Handler with complete recording configuration
        let recording_config = RecordingConfig {
            bucket: "test-bucket".to_string(),
            region: "us-east-1".to_string(),
            endpoint: "https://s3.amazonaws.com".to_string(),
            access_key: "access_key".to_string(),
            secret_key: "secret_key".to_string(),
            prefix: "recordings/test".to_string(),
        };

        let handler_with_recording = LiveKitRoomHandler::new(
            "http://localhost:7880".to_string(),
            "test_key".to_string(),
            "test_secret".to_string(),
            Some(recording_config),
        )
        .unwrap();

        assert!(handler.recording_config.is_none());
        assert!(handler_with_recording.recording_config.is_some());
        assert_eq!(
            handler_with_recording
                .recording_config
                .as_ref()
                .unwrap()
                .bucket,
            "test-bucket"
        );
        assert_eq!(
            handler_with_recording
                .recording_config
                .as_ref()
                .unwrap()
                .prefix,
            "recordings/test"
        );
    }

    #[test]
    fn test_recording_path_construction_with_prefix() {
        let filepath = build_recording_filepath("recordings/prod", "abc-123");
        assert_eq!(filepath, "recordings/prod/abc-123/audio.ogg");
    }

    #[test]
    fn test_recording_path_construction_empty_prefix() {
        let filepath = build_recording_filepath("", "abc-123");
        assert_eq!(filepath, "abc-123/audio.ogg");
    }

    #[test]
    fn test_recording_path_construction_prefix_with_trailing_slash() {
        let filepath = build_recording_filepath("recordings/prod/", "abc-123");
        assert_eq!(filepath, "recordings/prod/abc-123/audio.ogg");
    }

    #[test]
    fn test_recording_path_construction_multiple_trailing_slashes() {
        let filepath = build_recording_filepath("data///", "xyz");
        assert_eq!(filepath, "data/xyz/audio.ogg");
    }

    #[test]
    fn test_recording_path_construction_uuid_stream_id() {
        let filepath = build_recording_filepath("calls", "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(
            filepath,
            "calls/550e8400-e29b-41d4-a716-446655440000/audio.ogg"
        );
    }

    #[tokio::test]
    async fn test_setup_room_recording_without_config() {
        let handler = LiveKitRoomHandler::new(
            "http://localhost:7880".to_string(),
            "test_key".to_string(),
            "test_secret".to_string(),
            None,
        )
        .unwrap();

        let result = handler
            .setup_room_recording("test-room", "stream-123")
            .await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Recording configuration not provided")
        );
    }

    #[tokio::test]
    async fn test_setup_room_recording_with_config() {
        // This test validates that setup_room_recording accepts a properly configured handler
        // It will fail during actual API call since we don't have a real LiveKit server
        let recording_config = RecordingConfig {
            bucket: "test-bucket".to_string(),
            region: "us-east-1".to_string(),
            endpoint: "https://s3.amazonaws.com".to_string(),
            access_key: "access_key".to_string(),
            secret_key: "secret_key".to_string(),
            prefix: "recordings/test".to_string(),
        };

        let handler = LiveKitRoomHandler::new(
            "http://localhost:7880".to_string(),
            "test_key".to_string(),
            "test_secret".to_string(),
            Some(recording_config),
        )
        .unwrap();

        // This will fail at the API call stage, but that's expected since we don't have a real server
        // We're just validating that the configuration is accepted
        let result = handler
            .setup_room_recording("test-room", "stream-123")
            .await;

        // We expect an error because there's no real LiveKit server, but it shouldn't be a config error
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(!err_msg.contains("Recording configuration not provided"));
    }

    #[tokio::test]
    async fn test_list_participants_with_invalid_server() {
        // This test validates that list_participants fails gracefully when server is unreachable
        let handler = LiveKitRoomHandler::new(
            "http://localhost:7880".to_string(),
            "test_key".to_string(),
            "test_secret".to_string(),
            None,
        )
        .unwrap();

        let result = handler.list_participants("test-room").await;

        // We expect an error because there's no real LiveKit server
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Failed to list participants"));
    }

    #[tokio::test]
    async fn test_list_rooms_with_invalid_server() {
        // This test validates that list_rooms fails gracefully when server is unreachable
        let handler = LiveKitRoomHandler::new(
            "http://localhost:7880".to_string(),
            "test_key".to_string(),
            "test_secret".to_string(),
            None,
        )
        .unwrap();

        let result = handler.list_rooms(None).await;

        // We expect an error because there's no real LiveKit server
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Failed to list rooms"));
    }

    #[tokio::test]
    async fn test_list_rooms_with_prefix_invalid_server() {
        // This test validates that list_rooms with prefix fails gracefully when server is unreachable
        let handler = LiveKitRoomHandler::new(
            "http://localhost:7880".to_string(),
            "test_key".to_string(),
            "test_secret".to_string(),
            None,
        )
        .unwrap();

        let result = handler.list_rooms(Some("project1_")).await;

        // We expect an error because there's no real LiveKit server
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Failed to list rooms"));
    }

    #[tokio::test]
    async fn test_remove_participant_with_invalid_server() {
        // This test validates that remove_participant fails gracefully when server is unreachable
        let handler = LiveKitRoomHandler::new(
            "http://localhost:7880".to_string(),
            "test_key".to_string(),
            "test_secret".to_string(),
            None,
        )
        .unwrap();

        let result = handler
            .remove_participant("test-room", "test-participant")
            .await;

        // We expect an error because there's no real LiveKit server
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Failed to remove participant"));
    }

    #[tokio::test]
    async fn test_get_room_details_with_invalid_server() {
        // This test validates that get_room_details fails gracefully when server is unreachable
        let handler = LiveKitRoomHandler::new(
            "http://localhost:7880".to_string(),
            "test_key".to_string(),
            "test_secret".to_string(),
            None,
        )
        .unwrap();

        let result = handler.get_room_details("test-room").await;

        // We expect an error because there's no real LiveKit server
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Failed to get room"));
    }

    #[tokio::test]
    async fn test_update_room_metadata_with_invalid_server() {
        // This test validates that update_room_metadata fails gracefully when server is unreachable
        let handler = LiveKitRoomHandler::new(
            "http://localhost:7880".to_string(),
            "test_key".to_string(),
            "test_secret".to_string(),
            None,
        )
        .unwrap();

        let result = handler
            .update_room_metadata("test-room", r#"{"auth_id": "tenant-123"}"#)
            .await;

        // We expect an error because there's no real LiveKit server
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Failed to update room metadata"));
    }

    #[tokio::test]
    async fn test_ensure_room_auth_id_with_invalid_server() {
        // This test validates that ensure_room_auth_id fails gracefully when server is unreachable
        let handler = LiveKitRoomHandler::new(
            "http://localhost:7880".to_string(),
            "test_key".to_string(),
            "test_secret".to_string(),
            None,
        )
        .unwrap();

        let result = handler.ensure_room_auth_id("test-room", "tenant-123").await;

        // We expect an error because there's no real LiveKit server
        assert!(result.is_err());
        // The error should be about failing to get the room (first operation)
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Failed to get room"));
    }

    #[tokio::test]
    async fn test_ensure_room_with_auth_id_with_invalid_server() {
        // This test validates that ensure_room_with_auth_id fails gracefully when server is unreachable
        let handler = LiveKitRoomHandler::new(
            "http://localhost:7880".to_string(),
            "test_key".to_string(),
            "test_secret".to_string(),
            None,
        )
        .unwrap();

        let result = handler
            .ensure_room_with_auth_id("test-room", "tenant-123")
            .await;

        // We expect an error because there's no real LiveKit server
        // After room creation attempt, it should try ensure_room_auth_id again
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_ensure_room_exists_with_invalid_server() {
        // This test validates that ensure_room_exists fails gracefully when server is unreachable
        let handler = LiveKitRoomHandler::new(
            "http://localhost:7880".to_string(),
            "test_key".to_string(),
            "test_secret".to_string(),
            None,
        )
        .unwrap();

        let result = handler.ensure_room_exists("test-room").await;

        // We expect an error because there's no real LiveKit server
        assert!(result.is_err());
    }

    // Unit tests for metadata error conversion in ensure_room_auth_id
    // The actual metadata logic is tested in the metadata module
    mod metadata_integration {
        use super::super::super::metadata::{MetadataError, merge_auth_id};
        use super::*;

        #[test]
        fn test_metadata_conflict_detection() {
            // Test that merge_auth_id correctly detects conflicts
            let result = merge_auth_id(r#"{"auth_id": "tenant-A"}"#, "tenant-B");
            match result {
                Err(MetadataError::AuthIdConflict {
                    existing,
                    attempted,
                }) => {
                    assert_eq!(existing, "tenant-A");
                    assert_eq!(attempted, "tenant-B");
                }
                _ => panic!("Expected AuthIdConflict error"),
            }
        }

        #[test]
        fn test_metadata_preserves_existing_keys() {
            // Test that merge_auth_id preserves other metadata keys
            let result =
                merge_auth_id(r#"{"existing": "value", "count": 42}"#, "tenant-new").unwrap();
            let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
            assert_eq!(parsed["auth_id"], "tenant-new");
            assert_eq!(parsed["existing"], "value");
            assert_eq!(parsed["count"], 42);
        }

        #[test]
        fn test_metadata_idempotent_same_auth_id() {
            // Test that setting the same auth_id is idempotent
            let original = r#"{"auth_id": "tenant-same", "other": "data"}"#;
            let result = merge_auth_id(original, "tenant-same").unwrap();
            let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
            assert_eq!(parsed["auth_id"], "tenant-same");
            assert_eq!(parsed["other"], "data");
        }

        #[test]
        fn test_livekit_error_metadata_conflict_display() {
            // Test the display format of MetadataConflict error
            let err = LiveKitError::MetadataConflict {
                existing: "tenant-A".to_string(),
                attempted: "tenant-B".to_string(),
            };
            let msg = err.to_string();
            assert!(msg.contains("tenant-A"));
            assert!(msg.contains("tenant-B"));
            assert!(msg.contains("metadata conflict"));
        }

        #[test]
        fn test_livekit_error_room_not_found_display() {
            // Test the display format of RoomNotFound error
            let err = LiveKitError::RoomNotFound("my-room".to_string());
            let msg = err.to_string();
            assert!(msg.contains("my-room"));
            assert!(msg.contains("not found"));
        }
    }
}
