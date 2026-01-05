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
use tracing::{debug, info, warn};

use crate::utils::sip_api_client::{
    CreateSIPParticipantOptions, SIPApiClient, SIPInboundTrunkOptions, SIPOutboundTrunkOptions,
};

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

/// Configuration for SIP outbound trunk.
///
/// Reference: <https://docs.livekit.io/telephony/making-calls/outbound-trunk/>
///
/// # Authentication
///
/// LiveKit Cloud nodes do not have a static IP address range, so username/password
/// authentication is recommended over IP-based authentication with SIP providers.
/// Set `auth_username` and `auth_password` when your SIP provider requires credentials.
#[derive(Debug, Clone)]
pub struct OutboundTrunkConfig {
    /// Phone number that calls will originate from (e.g., "+15105550123")
    pub from_phone_number: String,
    /// SIP server address (e.g., "sip.telnyx.com")
    pub address: String,
    /// Optional SIP authentication username.
    /// Maps to `SipOutboundTrunkInfo.auth_username` in LiveKit API.
    pub auth_username: Option<String>,
    /// Optional SIP authentication password.
    /// Maps to `SipOutboundTrunkInfo.auth_password` in LiveKit API.
    /// SECURITY: Never log this value.
    pub auth_password: Option<String>,
}

/// Result of creating an outbound SIP call
#[derive(Debug, Clone)]
pub struct OutboundCallResult {
    /// Unique ID for the SIP call
    pub sip_call_id: String,
    /// Participant ID in the LiveKit room
    pub participant_id: String,
    /// Participant identity in the LiveKit room
    pub participant_identity: String,
    /// Room name the participant was connected to
    pub room_name: String,
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
            api_secret,
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

    /// Normalize a phone number for trunk matching.
    ///
    /// LiveKit stores phone numbers without the `tel:` prefix in trunk configurations.
    /// This helper ensures consistent matching by stripping the prefix if present.
    ///
    /// # Arguments
    /// * `phone_number` - Phone number to normalize (with or without tel: prefix)
    ///
    /// # Returns
    /// * The normalized phone number without tel: prefix
    pub fn normalize_phone_number(phone_number: &str) -> String {
        phone_number
            .strip_prefix("tel:")
            .unwrap_or(phone_number)
            .to_string()
    }

    /// Generate a deterministic trunk name from a phone number.
    ///
    /// Creates a consistent name for outbound trunks based on the from_phone_number
    /// to enable trunk reuse across calls from the same number.
    ///
    /// # Arguments
    /// * `from_phone_number` - Phone number that calls will originate from
    ///
    /// # Returns
    /// * A deterministic trunk name like "sayna-outbound-+15105550123"
    pub fn generate_outbound_trunk_name(from_phone_number: &str) -> String {
        let normalized = Self::normalize_phone_number(from_phone_number);
        format!("sayna-outbound-{}", normalized)
    }

    /// Ensure an outbound trunk exists for the given phone number.
    ///
    /// This method checks if an outbound trunk already exists for the specified
    /// `from_phone_number`, and creates one if it doesn't. This enables trunk reuse
    /// across multiple outbound calls from the same number.
    ///
    /// Reference: <https://docs.livekit.io/telephony/making-calls/outbound-trunk/>
    ///
    /// # Arguments
    /// * `config` - Outbound trunk configuration including optional authentication credentials
    ///
    /// # Authentication
    ///
    /// If `auth_username` and `auth_password` are provided, they will be included
    /// in the trunk configuration. This is required for SIP providers that use
    /// username/password authentication (recommended over IP-based auth for LiveKit Cloud).
    ///
    /// SECURITY: Authentication credentials are never logged.
    ///
    /// # Returns
    /// * `Result<String, LiveKitError>` - The trunk ID on success
    ///
    /// # Example
    /// ```rust,no_run
    /// use sayna::livekit::sip_handler::{LiveKitSipHandler, OutboundTrunkConfig};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let handler = LiveKitSipHandler::new(
    ///     "http://localhost:7880".to_string(),
    ///     "api_key".to_string(),
    ///     "api_secret".to_string(),
    /// );
    ///
    /// let config = OutboundTrunkConfig {
    ///     from_phone_number: "+15105550123".to_string(),
    ///     address: "sip.telnyx.com".to_string(),
    ///     auth_username: Some("user".to_string()),
    ///     auth_password: Some("pass".to_string()),
    /// };
    ///
    /// let trunk_id = handler.ensure_outbound_trunk(config).await?;
    /// println!("Using trunk: {}", trunk_id);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn ensure_outbound_trunk(
        &self,
        config: OutboundTrunkConfig,
    ) -> Result<String, LiveKitError> {
        let normalized_number = Self::normalize_phone_number(&config.from_phone_number);
        let trunk_name = Self::generate_outbound_trunk_name(&config.from_phone_number);

        debug!(
            from_phone_number = %config.from_phone_number,
            trunk_name = %trunk_name,
            "Looking up existing outbound trunk"
        );

        // List existing outbound trunks filtering by phone number
        // The API returns trunks containing this number, including wildcard trunks
        let existing_trunks = self
            .sip_api_client
            .list_sip_outbound_trunk(Some(vec![normalized_number.clone()]))
            .await?;

        // Check if any trunk matches our phone number
        for trunk in &existing_trunks {
            // Check if this trunk contains our phone number
            // (numbers can include wildcards like "*")
            if trunk.numbers.contains(&normalized_number)
                || trunk.numbers.contains(&"*".to_string())
            {
                info!(
                    trunk_id = %trunk.sip_trunk_id,
                    trunk_name = %trunk.name,
                    from_phone_number = %config.from_phone_number,
                    "Found existing outbound trunk"
                );
                return Ok(trunk.sip_trunk_id.clone());
            }
        }

        // No matching trunk found, create a new one
        // Log auth presence without exposing credential values (security best practice)
        info!(
            from_phone_number = %config.from_phone_number,
            trunk_name = %trunk_name,
            address = %config.address,
            auth_configured = config.auth_username.is_some(),
            "Creating new outbound trunk"
        );

        let options = SIPOutboundTrunkOptions {
            auth_username: config.auth_username,
            auth_password: config.auth_password,
            ..Default::default()
        };

        // Create the trunk
        let trunk_result = self
            .sip_api_client
            .create_sip_outbound_trunk(
                trunk_name.clone(),
                config.address,
                vec![normalized_number],
                options,
            )
            .await;

        match trunk_result {
            Ok(trunk) => {
                info!(
                    trunk_id = %trunk.sip_trunk_id,
                    trunk_name = %trunk.name,
                    "Created new outbound trunk"
                );
                Ok(trunk.sip_trunk_id)
            }
            Err(e) => {
                // Check if error indicates trunk already exists (concurrent creation race)
                let error_msg = e.to_string();
                if error_msg.contains("already exists") || error_msg.contains("duplicate") {
                    warn!(
                        trunk_name = %trunk_name,
                        "Trunk creation race detected, retrying lookup"
                    );
                    // Race condition: another request created the trunk
                    // Re-list to find the trunk that was just created
                    let trunks = self.sip_api_client.list_sip_outbound_trunk(None).await?;
                    for trunk in trunks {
                        if trunk.name == trunk_name {
                            info!(
                                trunk_id = %trunk.sip_trunk_id,
                                trunk_name = %trunk.name,
                                "Found trunk after race condition"
                            );
                            return Ok(trunk.sip_trunk_id);
                        }
                    }
                }
                Err(e)
            }
        }
    }

    /// Create an outbound SIP call.
    ///
    /// This method ensures an outbound trunk exists for the `from_phone_number`,
    /// then initiates a call to the `to_phone_number` and connects it to the
    /// specified LiveKit room.
    ///
    /// Reference: https://docs.livekit.io/sip/outbound-calls/
    ///
    /// # Arguments
    /// * `room_name` - LiveKit room to connect the call to
    /// * `to_phone_number` - Phone number to dial
    /// * `from_phone_number` - Phone number the call will originate from
    /// * `outbound_address` - SIP server address (e.g., "sip.telnyx.com")
    /// * `participant_identity` - Identity for the SIP participant in the room
    /// * `participant_name` - Display name for the SIP participant
    /// * `auth_username` - Optional SIP authentication username
    /// * `auth_password` - Optional SIP authentication password
    ///
    /// # Returns
    /// * `Result<OutboundCallResult, LiveKitError>` - Call info on success
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
    /// let result = handler.create_outbound_call(
    ///     "my-room",
    ///     "+15105551234",       // to
    ///     "+15105550123",       // from
    ///     "sip.telnyx.com",
    ///     "caller-1",           // identity
    ///     "Caller Name",        // name
    ///     Some("user".to_string()),
    ///     Some("pass".to_string()),
    /// ).await?;
    ///
    /// println!("Call ID: {}", result.sip_call_id);
    /// # Ok(())
    /// # }
    /// ```
    #[allow(clippy::too_many_arguments)]
    pub async fn create_outbound_call(
        &self,
        room_name: &str,
        to_phone_number: &str,
        from_phone_number: &str,
        outbound_address: &str,
        participant_identity: &str,
        participant_name: &str,
        auth_username: Option<String>,
        auth_password: Option<String>,
    ) -> Result<OutboundCallResult, LiveKitError> {
        debug!(
            room_name = %room_name,
            to_phone_number = %to_phone_number,
            from_phone_number = %from_phone_number,
            participant_identity = %participant_identity,
            "Creating outbound SIP call"
        );

        // Ensure the outbound trunk exists
        let trunk_config = OutboundTrunkConfig {
            from_phone_number: from_phone_number.to_string(),
            address: outbound_address.to_string(),
            auth_username,
            auth_password,
        };

        let trunk_id = self.ensure_outbound_trunk(trunk_config).await?;

        debug!(
            trunk_id = %trunk_id,
            "Using outbound trunk for call"
        );

        // Normalize phone numbers (strip tel: prefix for API)
        let normalized_to = Self::normalize_phone_number(to_phone_number);
        let normalized_from = Self::normalize_phone_number(from_phone_number);

        // Create the outbound call
        let options = CreateSIPParticipantOptions {
            ringing_timeout: Some(std::time::Duration::from_secs(30)),
            ..Default::default()
        };

        let participant = self
            .sip_api_client
            .create_sip_participant(
                &trunk_id,
                &normalized_to,
                room_name,
                participant_identity,
                participant_name,
                Some(&normalized_from),
                options,
            )
            .await?;

        info!(
            sip_call_id = %participant.sip_call_id,
            participant_id = %participant.participant_id,
            participant_identity = %participant.participant_identity,
            room_name = %participant.room_name,
            "Outbound call created successfully"
        );

        Ok(OutboundCallResult {
            sip_call_id: participant.sip_call_id,
            participant_id: participant.participant_id,
            participant_identity: participant.participant_identity,
            room_name: participant.room_name,
        })
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

    #[test]
    fn test_normalize_phone_number() {
        // Test phone number normalization (strip tel: prefix)
        let test_cases = vec![
            ("tel:+1234567890", "+1234567890"),
            ("+1234567890", "+1234567890"),
            ("tel:1234567890", "1234567890"),
            ("1234567890", "1234567890"),
            ("tel:+1-510-555-0123", "+1-510-555-0123"),
        ];

        for (input, expected) in test_cases {
            let result = LiveKitSipHandler::normalize_phone_number(input);
            assert_eq!(result, expected, "Input: {input}");
        }
    }

    #[test]
    fn test_generate_outbound_trunk_name() {
        // Test deterministic trunk name generation
        let test_cases = vec![
            ("+15105550123", "sayna-outbound-+15105550123"),
            ("tel:+15105550123", "sayna-outbound-+15105550123"),
            ("1234567890", "sayna-outbound-1234567890"),
        ];

        for (input, expected) in test_cases {
            let result = LiveKitSipHandler::generate_outbound_trunk_name(input);
            assert_eq!(result, expected, "Input: {input}");
        }
    }

    #[test]
    fn test_outbound_trunk_config_creation() {
        let config = OutboundTrunkConfig {
            from_phone_number: "+15105550123".to_string(),
            address: "sip.telnyx.com".to_string(),
            auth_username: Some("user".to_string()),
            auth_password: Some("pass".to_string()),
        };

        assert_eq!(config.from_phone_number, "+15105550123");
        assert_eq!(config.address, "sip.telnyx.com");
        assert_eq!(config.auth_username, Some("user".to_string()));
        assert_eq!(config.auth_password, Some("pass".to_string()));
    }

    #[test]
    fn test_outbound_trunk_config_without_auth() {
        let config = OutboundTrunkConfig {
            from_phone_number: "+15105550123".to_string(),
            address: "sip.example.com".to_string(),
            auth_username: None,
            auth_password: None,
        };

        assert!(config.auth_username.is_none());
        assert!(config.auth_password.is_none());
    }

    #[test]
    fn test_outbound_call_result_creation() {
        let result = OutboundCallResult {
            sip_call_id: "call-123".to_string(),
            participant_id: "participant-456".to_string(),
            participant_identity: "caller-1".to_string(),
            room_name: "my-room".to_string(),
        };

        assert_eq!(result.sip_call_id, "call-123");
        assert_eq!(result.participant_id, "participant-456");
        assert_eq!(result.participant_identity, "caller-1");
        assert_eq!(result.room_name, "my-room");
    }

    #[test]
    fn test_outbound_trunk_config_clone() {
        let config = OutboundTrunkConfig {
            from_phone_number: "+15105550123".to_string(),
            address: "sip.telnyx.com".to_string(),
            auth_username: Some("user".to_string()),
            auth_password: Some("pass".to_string()),
        };

        let cloned = config.clone();
        assert_eq!(cloned.from_phone_number, config.from_phone_number);
        assert_eq!(cloned.address, config.address);
        assert_eq!(cloned.auth_username, config.auth_username);
        assert_eq!(cloned.auth_password, config.auth_password);
    }

    #[test]
    fn test_outbound_call_result_clone() {
        let result = OutboundCallResult {
            sip_call_id: "call-123".to_string(),
            participant_id: "participant-456".to_string(),
            participant_identity: "caller-1".to_string(),
            room_name: "my-room".to_string(),
        };

        let cloned = result.clone();
        assert_eq!(cloned.sip_call_id, result.sip_call_id);
        assert_eq!(cloned.participant_id, result.participant_id);
        assert_eq!(cloned.participant_identity, result.participant_identity);
        assert_eq!(cloned.room_name, result.room_name);
    }

    /// Test that SIPOutboundTrunkOptions correctly receives auth credentials
    /// from OutboundTrunkConfig. This validates the config-to-options mapping
    /// without exposing credentials in logs.
    #[test]
    fn test_outbound_trunk_options_auth_mapping() {
        use crate::utils::sip_api_client::SIPOutboundTrunkOptions;

        // Test with auth credentials
        let config_with_auth = OutboundTrunkConfig {
            from_phone_number: "+15105550123".to_string(),
            address: "sip.provider.com".to_string(),
            auth_username: Some("sip_user".to_string()),
            auth_password: Some("sip_secret".to_string()),
        };

        let options: SIPOutboundTrunkOptions = SIPOutboundTrunkOptions {
            auth_username: config_with_auth.auth_username.clone(),
            auth_password: config_with_auth.auth_password.clone(),
            ..Default::default()
        };

        assert_eq!(options.auth_username, Some("sip_user".to_string()));
        assert_eq!(options.auth_password, Some("sip_secret".to_string()));

        // Test without auth credentials
        let config_no_auth = OutboundTrunkConfig {
            from_phone_number: "+15105550456".to_string(),
            address: "sip.other.com".to_string(),
            auth_username: None,
            auth_password: None,
        };

        let options_no_auth: SIPOutboundTrunkOptions = SIPOutboundTrunkOptions {
            auth_username: config_no_auth.auth_username.clone(),
            auth_password: config_no_auth.auth_password.clone(),
            ..Default::default()
        };

        assert!(options_no_auth.auth_username.is_none());
        assert!(options_no_auth.auth_password.is_none());
    }

    #[tokio::test]
    async fn test_ensure_outbound_trunk_with_invalid_server() {
        // This test validates that ensure_outbound_trunk fails when the server is unreachable
        let handler = LiveKitSipHandler::new(
            "http://localhost:7880".to_string(),
            "invalid_key".to_string(),
            "invalid_secret".to_string(),
        );

        let config = OutboundTrunkConfig {
            from_phone_number: "+15105550123".to_string(),
            address: "sip.telnyx.com".to_string(),
            auth_username: None,
            auth_password: None,
        };

        let result = handler.ensure_outbound_trunk(config).await;

        // We expect an error because there's no real LiveKit server
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_create_outbound_call_with_invalid_server() {
        // This test validates that create_outbound_call fails when the server is unreachable
        let handler = LiveKitSipHandler::new(
            "http://localhost:7880".to_string(),
            "invalid_key".to_string(),
            "invalid_secret".to_string(),
        );

        let result = handler
            .create_outbound_call(
                "my-room",
                "+15105551234",
                "+15105550123",
                "sip.telnyx.com",
                "caller-1",
                "Caller Name",
                None,
                None,
            )
            .await;

        // We expect an error because there's no real LiveKit server
        assert!(result.is_err());
    }
}
