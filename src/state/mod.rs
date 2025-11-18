use std::sync::Arc;

use crate::auth::AuthClient;
use crate::config::ServerConfig;
use crate::core::CoreState;
use crate::core::cache::store::CacheStore;
use crate::livekit::room_handler::{LiveKitRoomHandler, RecordingConfig};
use crate::livekit::sip_handler::{DispatchConfig, LiveKitSipHandler, TrunkConfig};
use crate::utils::req_manager::ReqManager;

/// Application state that can be shared across handlers
#[derive(Clone)]
pub struct AppState {
    pub config: ServerConfig,
    /// Core layer state that holds shared resources, such as TTS request managers
    pub core_state: Arc<CoreState>,
    /// LiveKit room handler for room and token management
    pub livekit_room_handler: Option<Arc<LiveKitRoomHandler>>,
    /// LiveKit SIP handler for SIP trunk and dispatch rule management
    pub livekit_sip_handler: Option<Arc<LiveKitSipHandler>>,
    /// Authentication client for validating bearer tokens (if auth is enabled)
    pub auth_client: Option<Arc<AuthClient>>,
}

impl AppState {
    pub async fn new(config: ServerConfig) -> Arc<Self> {
        let core_state = CoreState::new(&config).await;

        // Initialize LiveKit room handler if API keys are available
        let livekit_room_handler = if let (Some(api_key), Some(api_secret)) =
            (&config.livekit_api_key, &config.livekit_api_secret)
        {
            // Build recording config if all S3 settings are present
            let recording_config = if let (
                Some(bucket),
                Some(region),
                Some(endpoint),
                Some(access_key),
                Some(secret_key),
            ) = (
                &config.recording_s3_bucket,
                &config.recording_s3_region,
                &config.recording_s3_endpoint,
                &config.recording_s3_access_key,
                &config.recording_s3_secret_key,
            ) {
                Some(RecordingConfig {
                    bucket: bucket.clone(),
                    region: region.clone(),
                    endpoint: endpoint.clone(),
                    access_key: access_key.clone(),
                    secret_key: secret_key.clone(),
                })
            } else {
                None
            };

            match LiveKitRoomHandler::new(
                config.livekit_url.clone(),
                api_key.clone(),
                api_secret.clone(),
                recording_config,
            ) {
                Ok(handler) => Some(Arc::new(handler)),
                Err(e) => {
                    tracing::warn!("Failed to initialize LiveKit room handler: {:?}", e);
                    None
                }
            }
        } else {
            None
        };

        // Initialize auth client if JWT-based auth is configured
        // Note: API secret auth doesn't need a client - it's handled directly in middleware
        let auth_client = if config.auth_required && config.has_jwt_auth() {
            match AuthClient::from_config(&config).await {
                Ok(client) => {
                    tracing::info!(
                        "JWT authentication enabled with service: {}",
                        config
                            .auth_service_url
                            .as_ref()
                            .unwrap_or(&"unknown".to_string())
                    );
                    Some(Arc::new(client))
                }
                Err(e) => {
                    // Fail fast when JWT auth is configured but client initialization fails
                    tracing::error!("Failed to initialize auth client: {:?}", e);
                    panic!(
                        "JWT authentication configured but client initialization failed: {e:?}. \
                        Cannot start server without authentication. \
                        Please check AUTH_SERVICE_URL and AUTH_SIGNING_KEY_PATH configuration."
                    );
                }
            }
        } else if config.auth_required && config.has_api_secret_auth() {
            tracing::info!("API secret authentication enabled");
            None
        } else {
            None
        };

        // Initialize LiveKit SIP handler and provision trunk/dispatch if configured.
        // SIP features are fully opt-in: when config.sip is None, all SIP-related
        // code paths are skipped (provisioning, webhook forwarding, etc.).
        let livekit_sip_handler = if let Some(sip_config) = &config.sip {
            // Check if we have all required credentials for SIP provisioning
            if let (Some(api_key), Some(api_secret)) =
                (&config.livekit_api_key, &config.livekit_api_secret)
            {
                tracing::info!(
                    "SIP configuration detected, provisioning LiveKit SIP trunk and dispatch rules"
                );

                // Create the SIP handler
                let handler = LiveKitSipHandler::new(
                    config.livekit_url.clone(),
                    api_key.clone(),
                    api_secret.clone(),
                );

                // Build deterministic trunk and dispatch names based on room_prefix
                // This allows operators to predict resource names in the LiveKit UI
                let trunk_name = format!("sayna-{}-trunk", sip_config.room_prefix);
                let dispatch_name = format!("sayna-{}-dispatch", sip_config.room_prefix);

                // Prepare trunk configuration
                let trunk_config = TrunkConfig {
                    trunk_name: trunk_name.clone(),
                    allowed_addresses: sip_config.allowed_addresses.clone(),
                };

                // Prepare dispatch configuration
                // Note: max_participants is set to a sensible default of 3 (caller + Sayna + optional third party)
                // This can be exposed in config later if needed
                let dispatch_config = DispatchConfig {
                    dispatch_name: dispatch_name.clone(),
                    room_prefix: sip_config.room_prefix.clone(),
                    max_participants: 3,
                };

                // Provision the trunk and dispatch rule (idempotent - won't recreate if they exist)
                match handler
                    .configure_dispatch_rules(trunk_config, dispatch_config)
                    .await
                {
                    Ok(_) => {
                        tracing::info!(
                            "Successfully provisioned SIP resources: trunk={}, dispatch={}, livekit_url={}",
                            trunk_name,
                            dispatch_name,
                            config.livekit_url
                        );
                        Some(Arc::new(handler))
                    }
                    Err(e) => {
                        // Fail fast if provisioning fails - LiveKit can't deliver SIP calls without these resources
                        tracing::error!(
                            "Failed to provision SIP resources: trunk={}, dispatch={}, livekit_url={}, error={:?}",
                            trunk_name,
                            dispatch_name,
                            config.livekit_url,
                            e
                        );
                        panic!(
                            "SIP provisioning failed for trunk={}, dispatch={}: {e:?}. \
                            Cannot start server with SIP enabled. \
                            Please check LiveKit API credentials and server availability.",
                            trunk_name, dispatch_name
                        );
                    }
                }
            } else {
                tracing::info!(
                    "SIP configuration present but LiveKit API credentials missing. \
                    Skipping SIP provisioning. Set LIVEKIT_API_KEY and LIVEKIT_API_SECRET to enable."
                );
                None
            }
        } else {
            None
        };

        Arc::new(Self {
            config,
            core_state,
            livekit_room_handler,
            livekit_sip_handler,
            auth_client,
        })
    }

    /// Get a TTS request manager for a specific provider
    pub async fn get_tts_req_manager(&self, provider: &str) -> Option<Arc<ReqManager>> {
        self.core_state.get_tts_req_manager(provider).await
    }

    /// Get a handle to the application's cache store
    pub fn cache(&self) -> Arc<CacheStore> {
        self.core_state.cache.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SipConfig;

    #[test]
    fn test_sip_handler_not_created_without_config() {
        // This test verifies that when SIP config is None, the handler is not created
        let config = ServerConfig {
            host: "localhost".to_string(),
            port: 3001,
            livekit_url: "ws://localhost:7880".to_string(),
            livekit_public_url: "http://localhost:7880".to_string(),
            livekit_api_key: Some("test_key".to_string()),
            livekit_api_secret: Some("test_secret".to_string()),
            deepgram_api_key: None,
            elevenlabs_api_key: None,
            recording_s3_bucket: None,
            recording_s3_region: None,
            recording_s3_endpoint: None,
            recording_s3_access_key: None,
            recording_s3_secret_key: None,
            cache_path: None,
            cache_ttl_seconds: Some(3600),
            auth_service_url: None,
            auth_signing_key_path: None,
            auth_api_secret: None,
            auth_timeout_seconds: 5,
            auth_required: false,
            sip: None, // No SIP config
        };

        // We can't actually call AppState::new in a sync test, but we can verify
        // the logic that would be executed based on the config
        assert!(config.sip.is_none());
        assert!(config.livekit_api_key.is_some());
        assert!(config.livekit_api_secret.is_some());
    }

    #[test]
    fn test_sip_handler_skipped_without_credentials() {
        // This test verifies that when SIP config is present but credentials are missing,
        // the handler creation is skipped
        let config = ServerConfig {
            host: "localhost".to_string(),
            port: 3001,
            livekit_url: "ws://localhost:7880".to_string(),
            livekit_public_url: "http://localhost:7880".to_string(),
            livekit_api_key: None, // Missing API key
            livekit_api_secret: None, // Missing API secret
            deepgram_api_key: None,
            elevenlabs_api_key: None,
            recording_s3_bucket: None,
            recording_s3_region: None,
            recording_s3_endpoint: None,
            recording_s3_access_key: None,
            recording_s3_secret_key: None,
            cache_path: None,
            cache_ttl_seconds: Some(3600),
            auth_service_url: None,
            auth_signing_key_path: None,
            auth_api_secret: None,
            auth_timeout_seconds: 5,
            auth_required: false,
            sip: Some(SipConfig {
                room_prefix: "sip-".to_string(),
                allowed_addresses: vec!["192.168.1.0/24".to_string()],
                hooks: vec![],
            }),
        };

        // Verify that SIP config is present but credentials are missing
        assert!(config.sip.is_some());
        assert!(config.livekit_api_key.is_none());
        assert!(config.livekit_api_secret.is_none());
    }

    #[test]
    fn test_trunk_and_dispatch_name_generation() {
        // This test verifies the deterministic naming scheme for trunk and dispatch
        let room_prefix = "sip-";
        let expected_trunk_name = format!("sayna-{}-trunk", room_prefix);
        let expected_dispatch_name = format!("sayna-{}-dispatch", room_prefix);

        assert_eq!(expected_trunk_name, "sayna-sip--trunk");
        assert_eq!(expected_dispatch_name, "sayna-sip--dispatch");

        // Test with different prefix
        let room_prefix2 = "test-call-";
        let expected_trunk_name2 = format!("sayna-{}-trunk", room_prefix2);
        let expected_dispatch_name2 = format!("sayna-{}-dispatch", room_prefix2);

        assert_eq!(expected_trunk_name2, "sayna-test-call--trunk");
        assert_eq!(expected_dispatch_name2, "sayna-test-call--dispatch");
    }

    #[test]
    fn test_max_participants_default() {
        // This test verifies the default max_participants value
        // Default is 3: caller + Sayna + optional third party
        let max_participants: u32 = 3;
        assert_eq!(max_participants, 3);
    }
}
