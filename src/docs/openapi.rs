//! OpenAPI specification and documentation
//!
//! This module provides OpenAPI/Swagger documentation for the Sayna API.
//! It is only compiled when the `openapi` feature is enabled.

use axum::{routing::get, Json, Router};
use utoipa::OpenApi;

use crate::handlers::{
    api::HealthResponse,
    livekit::{TokenRequest, TokenResponse},
    speak::SpeakRequest,
    voices::Voice,
    ws::{
        config::{LiveKitWebSocketConfig, STTWebSocketConfig, TTSWebSocketConfig},
        messages::{IncomingMessage, OutgoingMessage, ParticipantDisconnectedInfo, UnifiedMessage},
    },
};
use crate::core::tts::Pronunciation;

/// OpenAPI documentation structure
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Sayna API",
        version = "0.1.0",
        description = "Real-time voice processing server with Speech-to-Text (STT) and Text-to-Speech (TTS) services",
        contact(
            name = "Sayna",
            url = "https://api.sayna.dev"
        )
    ),
    servers(
        (url = "https://api.sayna.dev", description = "Production API"),
        (url = "http://localhost:3001", description = "Local development")
    ),
    paths(
        crate::handlers::api::health_check,
        crate::handlers::voices::list_voices,
        crate::handlers::speak::speak_handler,
        crate::handlers::livekit::generate_token,
    ),
    components(schemas(
        // REST API types
        HealthResponse,
        Voice,
        SpeakRequest,
        TokenRequest,
        TokenResponse,
        // WebSocket message types
        IncomingMessage,
        OutgoingMessage,
        UnifiedMessage,
        ParticipantDisconnectedInfo,
        // Configuration types
        STTWebSocketConfig,
        TTSWebSocketConfig,
        LiveKitWebSocketConfig,
        Pronunciation,
    )),
    modifiers(&SecurityAddon),
    tags(
        (name = "health", description = "Health check endpoints"),
        (name = "voices", description = "TTS voice management"),
        (name = "tts", description = "Text-to-speech synthesis"),
        (name = "livekit", description = "LiveKit room and token management"),
        (name = "websocket", description = "WebSocket API for real-time communication")
    )
)]
pub struct ApiDoc;

/// Security scheme configuration
struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            let mut http = utoipa::openapi::security::Http::new(
                utoipa::openapi::security::HttpAuthScheme::Bearer,
            );
            http.bearer_format = Some("JWT".to_string());
            http.description = Some(
                "JWT token obtained from the authentication service. \
                 Required when AUTH_REQUIRED is enabled."
                    .to_string(),
            );

            components.add_security_scheme(
                "bearer_auth",
                utoipa::openapi::security::SecurityScheme::Http(http),
            )
        }
    }
}

/// Create OpenAPI documentation routes
///
/// Returns routes for serving the OpenAPI spec as JSON and YAML.
/// This should only be called when the `openapi` feature is enabled.
///
/// Routes:
/// - `GET /docs/openapi.json` - OpenAPI spec as JSON
/// - `GET /docs/openapi.yaml` - OpenAPI spec as YAML
pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/docs/openapi.json", get(openapi_json_handler))
        .route("/docs/openapi.yaml", get(openapi_yaml_handler))
}

/// Handler for GET /docs/openapi.json
async fn openapi_json_handler() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

/// Handler for GET /docs/openapi.yaml
async fn openapi_yaml_handler() -> ([(axum::http::header::HeaderName, &'static str); 1], String) {
    let yaml = spec_yaml().unwrap_or_else(|e| format!("Error generating YAML: {}", e));
    ([(axum::http::header::CONTENT_TYPE, "application/yaml")], yaml)
}

/// Get OpenAPI spec as YAML string
///
/// This is used for the CLI export command to generate docs/openapi.yaml
pub fn spec_yaml() -> Result<String, serde_yaml::Error> {
    serde_yaml::to_string(&ApiDoc::openapi())
}

/// Get OpenAPI spec as JSON string
pub fn spec_json() -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(&ApiDoc::openapi())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openapi_spec_generation() {
        // Ensure the OpenAPI spec can be generated without errors
        let spec = ApiDoc::openapi();
        assert_eq!(spec.info.title, "Sayna API");
        assert_eq!(spec.info.version, "0.1.0");
    }

    #[test]
    fn test_yaml_export() {
        // Ensure YAML export works
        let yaml = spec_yaml();
        assert!(yaml.is_ok());
        let yaml_str = yaml.unwrap();
        assert!(yaml_str.contains("Sayna API"));
    }

    #[test]
    fn test_json_export() {
        // Ensure JSON export works
        let json = spec_json();
        assert!(json.is_ok());
        let json_str = json.unwrap();
        assert!(json_str.contains("Sayna API"));
    }
}
