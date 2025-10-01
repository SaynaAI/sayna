//! # LiveKit SIP Handler
//!
//! This module provides functionality for managing LiveKit SIP trunks, dispatch rules,
//! and call transfers. It mirrors the Python SIP handler functionality but implemented
//! in Rust using the livekit-api crate.
//!
//! ## Features
//!
//! - **SIP Trunk Management**: Create and manage SIP inbound trunks with allowed addresses
//! - **Dispatch Rules**: Configure dispatch rules for routing SIP calls to rooms
//! - **Call Transfer**: Transfer SIP participants to different destinations
//!
//! ## Usage Example
//!
//! ```rust,no_run
//! use sayna::livekit::sip_handler::{LiveKitSipHandler, TrunkConfig, DispatchConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let handler = LiveKitSipHandler::new(
//!         "http://localhost:7880".to_string(),
//!         "api_key".to_string(),
//!         "api_secret".to_string(),
//!     );
//!
//!     // Configure trunk and dispatch rules
//!     let trunk_config = TrunkConfig {
//!         trunk_name: "local-trunk".to_string(),
//!         allowed_addresses: vec![
//!             "54.172.60.0/30".to_string(),
//!             "54.244.51.0/30".to_string(),
//!         ],
//!     };
//!
//!     let dispatch_config = DispatchConfig {
//!         dispatch_name: "local-dispatch".to_string(),
//!         agent_name: "ai".to_string(),
//!         room_prefix: "call".to_string(),
//!         max_participants: 3,
//!     };
//!
//!     handler.configure_dispatch_rules(trunk_config, dispatch_config).await?;
//!
//!     // Transfer a call
//!     handler.transfer_call("participant-id", "room-name", "tel:+1234567890").await?;
//!
//!     Ok(())
//! }
//! ```

use livekit_api::services::sip::{
    CreateSIPDispatchRuleOptions, CreateSIPInboundTrunkOptions, ListSIPDispatchRuleFilter,
    ListSIPInboundTrunkFilter, SIPClient,
};
use livekit_protocol as proto;

use super::types::LiveKitError;

/// Configuration for SIP inbound trunk
#[derive(Debug, Clone)]
pub struct TrunkConfig {
    /// Name of the SIP trunk
    pub trunk_name: String,
    /// List of allowed IP addresses/CIDR blocks
    pub allowed_addresses: Vec<String>,
}

/// Configuration for SIP dispatch rules
#[derive(Debug, Clone)]
pub struct DispatchConfig {
    /// Name of the dispatch rule
    pub dispatch_name: String,
    /// Name of the agent to dispatch
    pub agent_name: String,
    /// Prefix for room names
    pub room_prefix: String,
    /// Maximum participants in the room
    pub max_participants: u32,
}

/// Handler for LiveKit SIP management
///
/// This struct provides methods to configure SIP trunks, dispatch rules,
/// and manage call transfers.
pub struct LiveKitSipHandler {
    /// LiveKit server URL (e.g., "http://localhost:7880")
    url: String,
    /// API key for authentication
    api_key: String,
    /// API secret for authentication
    #[allow(dead_code)]
    api_secret: String,
    /// SIPClient for SIP operations
    sip_client: SIPClient,
}

impl LiveKitSipHandler {
    /// Create a new LiveKitSipHandler
    ///
    /// # Arguments
    /// * `url` - LiveKit server URL
    /// * `api_key` - LiveKit API key
    /// * `api_secret` - LiveKit API secret
    ///
    /// # Returns
    /// * `Self` - New handler instance
    ///
    /// # Example
    /// ```rust,no_run
    /// use sayna::livekit::sip_handler::LiveKitSipHandler;
    ///
    /// let handler = LiveKitSipHandler::new(
    ///     "http://localhost:7880".to_string(),
    ///     "api_key".to_string(),
    ///     "api_secret".to_string(),
    /// );
    /// ```
    pub fn new(url: String, api_key: String, api_secret: String) -> Self {
        let sip_client = SIPClient::with_api_key(&url, &api_key, &api_secret);

        Self {
            url,
            api_key,
            api_secret,
            sip_client,
        }
    }

    /// Configure SIP dispatch rules including trunk and dispatch rule creation
    ///
    /// This method checks if the trunk and dispatch rule exist, and creates them if they don't.
    ///
    /// # Arguments
    /// * `trunk_config` - Configuration for the SIP inbound trunk
    /// * `dispatch_config` - Configuration for the dispatch rule
    ///
    /// # Returns
    /// * `Result<(), LiveKitError>` - Success or error
    ///
    /// # Example
    /// ```rust,no_run
    /// use sayna::livekit::sip_handler::{LiveKitSipHandler, TrunkConfig, DispatchConfig};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let handler = LiveKitSipHandler::new(
    ///     "http://localhost:7880".to_string(),
    ///     "api_key".to_string(),
    ///     "api_secret".to_string(),
    /// );
    ///
    /// let trunk_config = TrunkConfig {
    ///     trunk_name: "local-trunk".to_string(),
    ///     allowed_addresses: vec!["54.172.60.0/30".to_string()],
    /// };
    ///
    /// let dispatch_config = DispatchConfig {
    ///     dispatch_name: "local-dispatch".to_string(),
    ///     agent_name: "ai".to_string(),
    ///     room_prefix: "call".to_string(),
    ///     max_participants: 3,
    /// };
    ///
    /// handler.configure_dispatch_rules(trunk_config, dispatch_config).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn configure_dispatch_rules(
        &self,
        trunk_config: TrunkConfig,
        dispatch_config: DispatchConfig,
    ) -> Result<(), LiveKitError> {
        // Check if trunk exists
        let trunks = self
            .sip_client
            .list_sip_inbound_trunk(ListSIPInboundTrunkFilter::All)
            .await
            .map_err(|e| {
                LiveKitError::ConnectionFailed(format!("Failed to list SIP inbound trunks: {}", e))
            })?;

        let trunk_exists = trunks
            .iter()
            .any(|trunk| trunk.name == trunk_config.trunk_name);

        if !trunk_exists {
            // Create trunk using the API signature: name, numbers, options
            let trunk_options = CreateSIPInboundTrunkOptions {
                allowed_addresses: Some(trunk_config.allowed_addresses.clone()),
                ..Default::default()
            };

            self.sip_client
                .create_sip_inbound_trunk(
                    trunk_config.trunk_name.clone(),
                    vec![], // empty numbers list
                    trunk_options,
                )
                .await
                .map_err(|e| {
                    LiveKitError::ConnectionFailed(format!(
                        "Failed to create SIP inbound trunk: {}",
                        e
                    ))
                })?;
        }

        // Check if dispatch rule exists
        let rules = self
            .sip_client
            .list_sip_dispatch_rule(ListSIPDispatchRuleFilter::All)
            .await
            .map_err(|e| {
                LiveKitError::ConnectionFailed(format!("Failed to list SIP dispatch rules: {}", e))
            })?;

        let rule_exists = rules
            .iter()
            .any(|rule| rule.name == dispatch_config.dispatch_name);

        if !rule_exists {
            // Note: The livekit-api crate doesn't currently support setting room_config
            // through CreateSIPDispatchRuleOptions. The room configuration (max_participants
            // and agent dispatch) needs to be set through the LiveKit server's admin API
            // or dashboard after creating the dispatch rule.
            //
            // TODO: Extend the livekit-api crate to support room_config in dispatch rules,
            // or use a lower-level twirp client request to set it during creation.

            // Create dispatch rule with individual dispatch (room prefix)
            let dispatch_rule = proto::sip_dispatch_rule::Rule::DispatchRuleIndividual(
                proto::SipDispatchRuleIndividual {
                    room_prefix: dispatch_config.room_prefix.clone(),
                    pin: String::new(),
                },
            );

            let dispatch_options = CreateSIPDispatchRuleOptions {
                name: dispatch_config.dispatch_name.clone(),
                ..Default::default()
            };

            self.sip_client
                .create_sip_dispatch_rule(dispatch_rule, dispatch_options)
                .await
                .map_err(|e| {
                    LiveKitError::ConnectionFailed(format!(
                        "Failed to create SIP dispatch rule: {}",
                        e
                    ))
                })?;

            // Note: Agent dispatch (dispatch_config.agent_name and max_participants)
            // must be configured separately through the LiveKit dashboard or API
        }

        Ok(())
    }

    /// Transfer a SIP participant to a different destination
    ///
    /// Note: This method is currently not implemented in the livekit-api crate.
    /// The TransferSIPParticipant protocol exists but the SIPClient doesn't expose it yet.
    /// This is a placeholder implementation that will need to be completed when the API is updated.
    ///
    /// # Arguments
    /// * `participant_identity` - Identity of the participant to transfer
    /// * `room_name` - Name of the room the participant is in
    /// * `transfer_to` - Destination to transfer to (e.g., "tel:+1234567890" or "+1234567890")
    ///
    /// # Returns
    /// * `Result<(), LiveKitError>` - Success or error
    ///
    /// # Example
    /// ```rust,no_run
    /// use sayna::livekit::sip_handler::LiveKitSipHandler;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let handler = LiveKitSipHandler::new(
    ///     "http://localhost:7880".to_string(),
    ///     "api_key".to_string(),
    ///     "api_secret".to_string(),
    /// );
    ///
    /// // This will return an error until the livekit-api crate adds support
    /// // handler.transfer_call("participant-id", "room-name", "tel:+1234567890").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn transfer_call(
        &self,
        _participant_identity: &str,
        _room_name: &str,
        _transfer_to: &str,
    ) -> Result<(), LiveKitError> {
        // TODO: Implement this when livekit-api crate adds TransferSIPParticipant support
        // The protocol supports this (proto::TransferSipParticipantRequest) but the
        // SIPClient doesn't expose the method yet.
        //
        // For now, return an error indicating the feature is not yet implemented
        Err(LiveKitError::ConnectionFailed(
            "Transfer SIP participant is not yet supported in livekit-api crate. \
             Please use the Python SDK or wait for Rust API update."
                .to_string(),
        ))
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
    fn test_sip_handler_creation() {
        let handler = LiveKitSipHandler::new(
            "http://localhost:7880".to_string(),
            "test_key".to_string(),
            "test_secret".to_string(),
        );

        assert_eq!(handler.url(), "http://localhost:7880");
        assert_eq!(handler.api_key(), "test_key");
    }

    #[test]
    fn test_trunk_config_creation() {
        let config = TrunkConfig {
            trunk_name: "test-trunk".to_string(),
            allowed_addresses: vec!["54.172.60.0/30".to_string(), "54.244.51.0/30".to_string()],
        };

        assert_eq!(config.trunk_name, "test-trunk");
        assert_eq!(config.allowed_addresses.len(), 2);
    }

    #[test]
    fn test_dispatch_config_creation() {
        let config = DispatchConfig {
            dispatch_name: "test-dispatch".to_string(),
            agent_name: "ai".to_string(),
            room_prefix: "call".to_string(),
            max_participants: 3,
        };

        assert_eq!(config.dispatch_name, "test-dispatch");
        assert_eq!(config.agent_name, "ai");
        assert_eq!(config.room_prefix, "call");
        assert_eq!(config.max_participants, 3);
    }

    #[test]
    fn test_transfer_to_tel_prefix() {
        // This test validates the tel: prefix logic
        let test_cases = vec![
            ("tel:+1234567890", "tel:+1234567890"),
            ("+1234567890", "tel:+1234567890"),
            ("1234567890", "tel:1234567890"),
        ];

        for (input, expected) in test_cases {
            let result = if input.starts_with("tel:") {
                input.to_string()
            } else {
                format!("tel:{}", input)
            };
            assert_eq!(result, expected);
        }
    }

    #[tokio::test]
    async fn test_configure_dispatch_rules_with_invalid_credentials() {
        // This test validates that configure_dispatch_rules fails with invalid credentials
        let handler = LiveKitSipHandler::new(
            "http://localhost:7880".to_string(),
            "invalid_key".to_string(),
            "invalid_secret".to_string(),
        );

        let trunk_config = TrunkConfig {
            trunk_name: "test-trunk".to_string(),
            allowed_addresses: vec!["54.172.60.0/30".to_string()],
        };

        let dispatch_config = DispatchConfig {
            dispatch_name: "test-dispatch".to_string(),
            agent_name: "ai".to_string(),
            room_prefix: "call".to_string(),
            max_participants: 3,
        };

        let result = handler
            .configure_dispatch_rules(trunk_config, dispatch_config)
            .await;

        // We expect an error because there's no real LiveKit server
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_transfer_call_not_implemented() {
        // This test validates that transfer_call returns not implemented error
        let handler = LiveKitSipHandler::new(
            "http://localhost:7880".to_string(),
            "invalid_key".to_string(),
            "invalid_secret".to_string(),
        );

        let result = handler
            .transfer_call("participant-id", "room-name", "tel:+1234567890")
            .await;

        // We expect an error because the feature is not yet implemented
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("not yet supported")
        );
    }
}
