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

use livekit_api::access_token::{AccessToken, VideoGrants};
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
    /// * `file_key` - Unique identifier for the recording stream
    ///
    /// # Returns
    /// * `Result<String, LiveKitError>` - Egress ID for the started recording or error
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
    ///     }),
    /// )?;
    ///
    /// let egress_id = handler.setup_room_recording("my-room", "stream-123").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn setup_room_recording(
        &self,
        room_name: &str,
        file_key: &str,
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

        // Create encoded file output with S3 upload
        let file_output = proto::EncodedFileOutput {
            file_type: proto::EncodedFileType::Ogg as i32,
            filepath: file_key.to_string(),
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
}
