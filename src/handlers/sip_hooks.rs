//! SIP hooks REST API handlers
//!
//! This module provides REST API endpoints for managing SIP webhook configurations
//! at runtime. Changes are persisted to the cache directory and merged with the
//! server configuration on startup.

use axum::{extract::State, http::StatusCode, response::Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{error, info, warn};

use crate::state::AppState;
use crate::utils::sip_hooks::{self, CachedSipHook};

/// Request body for updating SIP hooks.
///
/// Contains a list of SIP webhook configurations to add or replace.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SipHooksRequest {
    /// List of SIP hooks to add or replace.
    /// Hooks with matching hosts (case-insensitive) will be replaced.
    #[serde(default)]
    pub hooks: Vec<SipHookEntry>,
}

/// A single SIP hook entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SipHookEntry {
    /// Host pattern for matching SIP domains (case-insensitive)
    #[cfg_attr(feature = "openapi", schema(example = "example.com"))]
    pub host: String,

    /// HTTPS URL to forward webhook events to
    #[cfg_attr(
        feature = "openapi",
        schema(example = "https://webhook.example.com/events")
    )]
    pub url: String,
}

/// Response body for SIP hooks operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SipHooksResponse {
    /// List of all configured SIP hooks
    pub hooks: Vec<SipHookEntry>,
}

/// Error response for SIP hooks operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SipHooksErrorResponse {
    /// Error message describing what went wrong
    #[cfg_attr(
        feature = "openapi",
        schema(example = "Duplicate host detected: example.com")
    )]
    pub error: String,
}

/// Lists all configured SIP hooks.
///
/// Returns the current list of SIP hooks from the cache file.
/// This endpoint requires authentication if `AUTH_REQUIRED=true`.
///
/// # Returns
/// * `200 OK` - List of SIP hooks
/// * `500 Internal Server Error` - If reading the cache fails
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/sip/hooks",
        responses(
            (status = 200, description = "List of SIP hooks", body = SipHooksResponse),
            (status = 500, description = "Failed to read hooks cache", body = SipHooksErrorResponse)
        ),
        tag = "sip"
    )
)]
pub async fn list_sip_hooks(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SipHooksResponse>, (StatusCode, Json<SipHooksErrorResponse>)> {
    let cache_path = match &state.config.cache_path {
        Some(path) => path.clone(),
        None => {
            // No cache configured, return empty list
            return Ok(Json(SipHooksResponse { hooks: vec![] }));
        }
    };

    let cached_hooks = sip_hooks::read_hooks_cache(&cache_path)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to read SIP hooks cache");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(SipHooksErrorResponse {
                    error: format!("Failed to read hooks cache: {}", e),
                }),
            )
        })?;

    let hooks: Vec<SipHookEntry> = cached_hooks
        .into_iter()
        .map(|h| SipHookEntry {
            host: h.host,
            url: h.url,
        })
        .collect();

    Ok(Json(SipHooksResponse { hooks }))
}

/// Updates SIP hooks.
///
/// Adds or replaces SIP hooks in the cache. Hooks with matching hosts
/// (case-insensitive) will be replaced. The changes take effect immediately
/// and persist across server restarts.
///
/// **Note**: Secrets are NOT stored in the cache. Runtime-added hooks will
/// use the global `hook_secret` from the server configuration.
///
/// # Request Body
/// Array of hook entries with `host` and `url` fields.
///
/// # Returns
/// * `200 OK` - Updated list of SIP hooks
/// * `400 Bad Request` - If validation fails (e.g., duplicate hosts)
/// * `500 Internal Server Error` - If writing the cache fails
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/sip/hooks",
        request_body = SipHooksRequest,
        responses(
            (status = 200, description = "Updated list of SIP hooks", body = SipHooksResponse),
            (status = 400, description = "Validation error", body = SipHooksErrorResponse),
            (status = 500, description = "Failed to write hooks cache", body = SipHooksErrorResponse)
        ),
        tag = "sip"
    )
)]
pub async fn update_sip_hooks(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SipHooksRequest>,
) -> Result<Json<SipHooksResponse>, (StatusCode, Json<SipHooksErrorResponse>)> {
    let cache_path = match &state.config.cache_path {
        Some(path) => path.clone(),
        None => {
            warn!("No cache path configured, cannot persist SIP hooks");
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(SipHooksErrorResponse {
                    error: "No cache path configured. Cannot persist SIP hooks.".to_string(),
                }),
            ));
        }
    };

    // Convert request entries to CachedSipHook
    let new_hooks: Vec<CachedSipHook> = request
        .hooks
        .into_iter()
        .map(|h| CachedSipHook::new(h.host, h.url))
        .collect();

    // Validate for duplicates in the incoming request
    if let Err(e) = sip_hooks::validate_no_duplicate_hosts(&new_hooks) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(SipHooksErrorResponse {
                error: e.to_string(),
            }),
        ));
    }

    // Read existing hooks from cache
    let existing_hooks = sip_hooks::read_hooks_cache(&cache_path)
        .await
        .unwrap_or_default();

    // Merge: new hooks override existing (case-insensitive host match)
    let merged_hooks = sip_hooks::merge_hooks(&existing_hooks, &new_hooks);

    // Write merged hooks to cache
    sip_hooks::write_hooks_cache(&cache_path, &merged_hooks)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to write SIP hooks cache");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(SipHooksErrorResponse {
                    error: format!("Failed to write hooks cache: {}", e),
                }),
            )
        })?;

    info!(
        hook_count = merged_hooks.len(),
        new_hooks = new_hooks.len(),
        "Updated SIP hooks cache"
    );

    let hooks: Vec<SipHookEntry> = merged_hooks
        .into_iter()
        .map(|h| SipHookEntry {
            host: h.host,
            url: h.url,
        })
        .collect();

    Ok(Json(SipHooksResponse { hooks }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sip_hook_entry_serialization() {
        let entry = SipHookEntry {
            host: "example.com".to_string(),
            url: "https://webhook.example.com/events".to_string(),
        };

        let json = serde_json::to_string(&entry).expect("Failed to serialize");
        assert!(json.contains("example.com"));
        assert!(json.contains("https://webhook.example.com/events"));
    }

    #[test]
    fn test_sip_hooks_request_deserialization() {
        let json =
            r#"{"hooks": [{"host": "example.com", "url": "https://webhook.example.com/events"}]}"#;
        let request: SipHooksRequest = serde_json::from_str(json).expect("Failed to deserialize");

        assert_eq!(request.hooks.len(), 1);
        assert_eq!(request.hooks[0].host, "example.com");
    }

    #[test]
    fn test_sip_hooks_request_empty_hooks() {
        let json = r#"{"hooks": []}"#;
        let request: SipHooksRequest = serde_json::from_str(json).expect("Failed to deserialize");
        assert!(request.hooks.is_empty());
    }

    #[test]
    fn test_sip_hooks_request_default() {
        let json = r#"{}"#;
        let request: SipHooksRequest = serde_json::from_str(json).expect("Failed to deserialize");
        assert!(request.hooks.is_empty());
    }

    #[test]
    fn test_sip_hooks_response_serialization() {
        let response = SipHooksResponse {
            hooks: vec![SipHookEntry {
                host: "example.com".to_string(),
                url: "https://webhook.example.com/events".to_string(),
            }],
        };

        let json = serde_json::to_string(&response).expect("Failed to serialize");
        assert!(json.contains("hooks"));
        assert!(json.contains("example.com"));
    }
}
