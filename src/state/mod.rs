use std::sync::Arc;

use crate::config::ServerConfig;
use crate::core::CoreState;
use crate::core::cache::store::CacheStore;
use crate::livekit::room_handler::{LiveKitRoomHandler, RecordingConfig};
use crate::utils::req_manager::ReqManager;

/// Application state that can be shared across handlers
#[derive(Clone)]
pub struct AppState {
    pub config: ServerConfig,
    /// Core layer state that holds shared resources, such as TTS request managers
    pub core_state: Arc<CoreState>,
    /// LiveKit room handler for room and token management
    pub livekit_room_handler: Option<Arc<LiveKitRoomHandler>>,
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

        Arc::new(Self {
            config,
            core_state,
            livekit_room_handler,
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
