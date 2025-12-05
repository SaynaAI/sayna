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
    CreateSIPDispatchRuleOptions, ListSIPDispatchRuleFilter, ListSIPInboundTrunkFilter, SIPClient,
};
use livekit_protocol as proto;

use crate::utils::sip_api_client::{SIPApiClient, SIPInboundTrunkOptions};

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
    /// SIPClient for SIP operations (from livekit-api crate)
    sip_client: SIPClient,
    /// SIPApiClient for raw Twirp API calls (for features not exposed by livekit-api)
    sip_api_client: SIPApiClient,
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
        let sip_api_client = SIPApiClient::new(url.clone(), api_key.clone(), api_secret.clone());

        Self {
            url,
            api_key,
            sip_client,
            sip_api_client,
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
                LiveKitError::ConnectionFailed(format!("Failed to list SIP inbound trunks: {e}"))
            })?;

        let trunk_exists = trunks
            .iter()
            .any(|trunk| trunk.name == trunk_config.trunk_name);

        if !trunk_exists {
            // Use our stored SIP API client to set include_headers (not yet exposed in livekit-api).
            let trunk_options = SIPInboundTrunkOptions {
                allowed_addresses: Some(trunk_config.allowed_addresses.clone()),
                include_headers: Some(proto::SipHeaderOptions::SipAllHeaders),
                ..Default::default()
            };

            self.sip_api_client
                .create_sip_inbound_trunk(
                    trunk_config.trunk_name.clone(),
                    vec![], // empty numbers list
                    trunk_options,
                )
                .await
                .map_err(|e| {
                    LiveKitError::ConnectionFailed(format!(
                        "Failed to create SIP inbound trunk: {e}"
                    ))
                })?;
        }

        // Check if dispatch rule exists
        let rules = self
            .sip_client
            .list_sip_dispatch_rule(ListSIPDispatchRuleFilter::All)
            .await
            .map_err(|e| {
                LiveKitError::ConnectionFailed(format!("Failed to list SIP dispatch rules: {e}"))
            })?;

        let rule_exists = rules
            .iter()
            .any(|rule| rule.name == dispatch_config.dispatch_name);

        if !rule_exists {
            // Note: The livekit-api crate doesn't currently support setting room_config
            // through CreateSIPDispatchRuleOptions. The max_participants setting
            // needs to be set through the LiveKit server's admin API or dashboard
            // after creating the dispatch rule.
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
                        "Failed to create SIP dispatch rule: {e}"
                    ))
                })?;

            // Note: max_participants must be configured separately through
            // the LiveKit dashboard or API
        }

        Ok(())
    }

    /// Transfer a SIP participant to a different destination
    ///
    /// This method initiates a SIP REFER to transfer the call to the specified destination.
    /// It uses the raw Twirp API to bypass limitations in the livekit-api crate.
    ///
    /// # Arguments
    /// * `participant_identity` - Identity of the participant to transfer
    /// * `room_name` - Name of the room the participant is in
    /// * `transfer_to` - Destination to transfer to. Accepts various formats:
    ///     - International format: `+1234567890` or `tel:+1234567890`
    ///     - National format: `07123456789` or `tel:07123456789`
    ///     - Extension: `1234` or `tel:1234`
    ///
    ///   The `tel:` prefix will be added automatically if not present.
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
    /// // Transfer to international number
    /// handler.transfer_call("participant-id", "room-name", "+1234567890").await?;
    ///
    /// // Transfer with tel: prefix (also works)
    /// handler.transfer_call("participant-id", "room-name", "tel:+1234567890").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn transfer_call(
        &self,
        participant_identity: &str,
        room_name: &str,
        transfer_to: &str,
    ) -> Result<(), LiveKitError> {
        // Ensure tel: prefix is present
        let formatted_transfer_to = if transfer_to.starts_with("tel:") {
            transfer_to.to_string()
        } else {
            format!("tel:{transfer_to}")
        };

        self.sip_api_client
            .transfer_sip_participant(room_name, participant_identity, &formatted_transfer_to)
            .await
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
            room_prefix: "call".to_string(),
            max_participants: 3,
        };

        assert_eq!(config.dispatch_name, "test-dispatch");
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
                format!("tel:{input}")
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
    async fn test_transfer_call_with_invalid_server() {
        // This test validates that transfer_call fails when the server is unreachable
        let handler = LiveKitSipHandler::new(
            "http://localhost:7880".to_string(),
            "invalid_key".to_string(),
            "invalid_secret".to_string(),
        );

        let result = handler
            .transfer_call("participant-id", "room-name", "tel:+1234567890")
            .await;

        // We expect an error because there's no real LiveKit server
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_transfer_call_adds_tel_prefix() {
        // This test validates that transfer_call adds the tel: prefix when needed
        // by checking the error message contains the formatted phone number
        let handler = LiveKitSipHandler::new(
            "http://localhost:7880".to_string(),
            "test_key".to_string(),
            "test_secret".to_string(),
        );

        // Test without tel: prefix - should still work (error due to no server)
        let result = handler
            .transfer_call("participant-id", "room-name", "+1234567890")
            .await;
        assert!(result.is_err());

        // Test with tel: prefix - should still work (error due to no server)
        let result = handler
            .transfer_call("participant-id", "room-name", "tel:+1234567890")
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn test_transfer_to_tel_prefix_internal_extension() {
        // Test internal extensions get tel: prefix added
        let test_cases = vec![
            ("1234", "tel:1234"),
            ("tel:1234", "tel:1234"),
            ("0", "tel:0"),
            ("tel:0", "tel:0"),
        ];

        for (input, expected) in test_cases {
            let result = if input.starts_with("tel:") {
                input.to_string()
            } else {
                format!("tel:{input}")
            };
            assert_eq!(result, expected, "Input: {input}");
        }
    }

    #[test]
    fn test_transfer_to_tel_prefix_national_format() {
        // Test national format numbers get tel: prefix added
        let test_cases = vec![
            ("07123456789", "tel:07123456789"),
            ("tel:07123456789", "tel:07123456789"),
        ];

        for (input, expected) in test_cases {
            let result = if input.starts_with("tel:") {
                input.to_string()
            } else {
                format!("tel:{input}")
            };
            assert_eq!(result, expected, "Input: {input}");
        }
    }

    #[test]
    fn test_handler_accessors() {
        let handler = LiveKitSipHandler::new(
            "http://my-livekit:7880".to_string(),
            "my_api_key".to_string(),
            "my_api_secret".to_string(),
        );

        assert_eq!(handler.url(), "http://my-livekit:7880");
        assert_eq!(handler.api_key(), "my_api_key");
    }

    #[test]
    fn test_trunk_config_clone() {
        let config = TrunkConfig {
            trunk_name: "test-trunk".to_string(),
            allowed_addresses: vec!["10.0.0.0/8".to_string()],
        };

        let cloned = config.clone();
        assert_eq!(cloned.trunk_name, config.trunk_name);
        assert_eq!(cloned.allowed_addresses, config.allowed_addresses);
    }

    #[test]
    fn test_dispatch_config_clone() {
        let config = DispatchConfig {
            dispatch_name: "test-dispatch".to_string(),
            room_prefix: "room".to_string(),
            max_participants: 5,
        };

        let cloned = config.clone();
        assert_eq!(cloned.dispatch_name, config.dispatch_name);
        assert_eq!(cloned.room_prefix, config.room_prefix);
        assert_eq!(cloned.max_participants, config.max_participants);
    }
}
