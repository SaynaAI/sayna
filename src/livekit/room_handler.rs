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
//!     )?;
//!
//!     // Create a room
//!     handler.create_room("my-room").await?;
//!
//!     // Generate user token
//!     let user_token = handler.user_token("my-room")?;
//!
//!     // Generate agent token with admin privileges
//!     let agent_token = handler.agent_token("my-room")?;
//!
//!     Ok(())
//! }
//! ```

use livekit_api::access_token::{AccessToken, VideoGrants};
use livekit_api::services::room::{CreateRoomOptions, RoomClient};

use super::types::LiveKitError;

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
}

impl LiveKitRoomHandler {
    /// Create a new LiveKitRoomHandler
    ///
    /// # Arguments
    /// * `url` - LiveKit server URL
    /// * `api_key` - LiveKit API key
    /// * `api_secret` - LiveKit API secret
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
    /// )?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn new(url: String, api_key: String, api_secret: String) -> Result<Self, LiveKitError> {
        let room_client = RoomClient::with_api_key(&url, &api_key, &api_secret);

        Ok(Self {
            url,
            api_key,
            api_secret,
            room_client,
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
    /// )?;
    ///
    /// let token = handler.user_token("my-room")?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn user_token(&self, room_name: &str) -> Result<String, LiveKitError> {
        let token = AccessToken::with_api_key(&self.api_key, &self.api_secret)
            .with_identity("user")
            .with_name("user")
            .with_grants(self.token_permissions(room_name, false))
            .to_jwt()
            .map_err(|e| {
                LiveKitError::ConnectionFailed(format!("Failed to generate user token: {}", e))
            })?;

        Ok(token)
    }

    /// Generate a JWT token for an agent with admin privileges
    ///
    /// # Arguments
    /// * `room_name` - Name of the LiveKit room
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
    /// )?;
    ///
    /// let token = handler.agent_token("my-room")?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn agent_token(&self, room_name: &str) -> Result<String, LiveKitError> {
        let token = AccessToken::with_api_key(&self.api_key, &self.api_secret)
            .with_identity("ai")
            .with_name("ai")
            .with_grants(self.token_permissions(room_name, true))
            .to_jwt()
            .map_err(|e| {
                LiveKitError::ConnectionFailed(format!("Failed to generate agent token: {}", e))
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
    /// )?;
    ///
    /// handler.create_room("my-room").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn create_room(&self, room_name: &str) -> Result<(), LiveKitError> {
        let mut options = CreateRoomOptions::default();
        options.max_participants = 3;

        self.room_client
            .create_room(room_name, options)
            .await
            .map_err(|e| LiveKitError::ConnectionFailed(format!("Failed to create room: {}", e)))?;

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
        )
        .unwrap();

        let result = handler.user_token("test-room");
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
        )
        .unwrap();

        let result = handler.agent_token("test-room");
        assert!(result.is_ok());
        let token = result.unwrap();
        assert!(!token.is_empty());
    }
}
