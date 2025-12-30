//! SIP hooks REST API handlers
//!
//! This module provides REST API endpoints for managing SIP webhook configurations
//! at runtime. Changes are persisted to the cache directory and merged with the
//! server configuration on startup.

use axum::{extract::State, http::StatusCode, response::Json};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
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

/// Returns the set of hosts defined in application configuration.
fn configured_sip_hosts(state: &AppState) -> HashSet<String> {
    state
        .config
        .sip
        .as_ref()
        .map(|sip| {
            sip.hooks
                .iter()
                .map(|hook| hook.host.trim().to_lowercase())
                .collect()
        })
        .unwrap_or_default()
}

/// Reject updates that attempt to touch hosts hardcoded in application config.
fn reject_protected_hosts<'a, I>(
    protected_hosts: &HashSet<String>,
    hosts: I,
) -> Option<(StatusCode, Json<SipHooksErrorResponse>)>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut blocked: Vec<String> = hosts
        .into_iter()
        .filter_map(|host| {
            let normalized = host.trim().to_lowercase();
            protected_hosts.contains(&normalized).then_some(normalized)
        })
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    if blocked.is_empty() {
        return None;
    }

    blocked.sort();

    Some((
        StatusCode::METHOD_NOT_ALLOWED,
        Json(SipHooksErrorResponse {
            error: format!(
                "Host(s) {} are hardcoded in the application config and cannot be changed.",
                blocked.join(", ")
            ),
        }),
    ))
}

/// Lists all configured SIP hooks.
///
/// Returns the current list of SIP hooks from runtime state.
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
    if let Some(hooks_state) = state.core_state.get_sip_hooks_state() {
        let hooks_guard = hooks_state.read().await;
        let hooks: Vec<SipHookEntry> = hooks_guard
            .get_hooks()
            .iter()
            .map(|h| SipHookEntry {
                host: h.host.clone(),
                url: h.url.clone(),
            })
            .collect();

        return Ok(Json(SipHooksResponse { hooks }));
    }

    Ok(Json(SipHooksResponse { hooks: vec![] }))
}

/// Updates SIP hooks.
///
/// Adds or replaces SIP hooks in the cache. Hooks with matching hosts
/// (case-insensitive) will be replaced. Hosts defined in the application
/// configuration cannot be modified. The changes take effect immediately
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
            (status = 405, description = "Host defined in application config cannot be modified", body = SipHooksErrorResponse),
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

    let protected_hosts = configured_sip_hosts(&state);

    // Convert request entries to CachedSipHook
    let new_hooks: Vec<CachedSipHook> = request
        .hooks
        .into_iter()
        .map(|h| CachedSipHook::new(h.host, h.url))
        .collect();

    if let Some(error) = reject_protected_hosts(
        &protected_hosts,
        new_hooks.iter().map(|hook| hook.host.as_str()),
    ) {
        return Err(error);
    }

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

    let filtered_existing_hooks: Vec<CachedSipHook> = existing_hooks
        .into_iter()
        .filter(|hook| !protected_hosts.contains(&hook.host))
        .collect();

    // Merge: new hooks override existing (case-insensitive host match)
    let merged_hooks = sip_hooks::merge_hooks(&filtered_existing_hooks, &new_hooks);

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

    // Update runtime state and get the fully merged hooks (config + cache)
    let response_hooks: Vec<SipHookEntry> =
        if let Some(hooks_state) = state.core_state.get_sip_hooks_state() {
            let mut hooks_guard = hooks_state.write().await;
            let original_hooks = state
                .config
                .sip
                .as_ref()
                .map(|c| c.hooks.as_slice())
                .unwrap_or(&[]);
            // update_hooks merges cache with config and returns the full state
            let full_state = hooks_guard.update_hooks(merged_hooks.clone(), original_hooks);

            info!(
                hook_count = full_state.len(),
                new_hooks = new_hooks.len(),
                "Updated SIP hooks cache and runtime state"
            );

            full_state
                .into_iter()
                .map(|h| SipHookEntry {
                    host: h.host,
                    url: h.url,
                })
                .collect()
        } else {
            // No SIP state configured, return the cache-only merge
            info!(
                hook_count = merged_hooks.len(),
                new_hooks = new_hooks.len(),
                "Updated SIP hooks cache (no runtime state)"
            );

            merged_hooks
                .into_iter()
                .map(|h| SipHookEntry {
                    host: h.host,
                    url: h.url,
                })
                .collect()
        };

    Ok(Json(SipHooksResponse {
        hooks: response_hooks,
    }))
}

/// Request body for deleting SIP hooks.
///
/// Contains a list of host names to remove from the SIP hooks cache.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeleteSipHooksRequest {
    /// List of host names to remove (case-insensitive).
    /// Hosts that exist in the original config will revert to their config values.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(example = json!(["example.com", "other.com"])))]
    pub hosts: Vec<String>,
}

/// Deletes SIP hooks by host name.
///
/// Removes the specified hosts from the cache. Hosts defined in the application
/// configuration cannot be removed. If a host exists in the original server
/// configuration, it will revert to its config value after deletion of a
/// cached override. Hosts that only exist in cache will be completely removed.
///
/// The changes take effect immediately and persist across server restarts.
///
/// # Request Body
/// Array of host names to remove (case-insensitive).
///
/// # Returns
/// * `200 OK` - Updated list of SIP hooks after deletion
/// * `400 Bad Request` - If the hosts array is empty
/// * `500 Internal Server Error` - If writing the cache fails
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        delete,
        path = "/sip/hooks",
        request_body = DeleteSipHooksRequest,
        responses(
            (status = 200, description = "Updated list of SIP hooks after deletion", body = SipHooksResponse),
            (status = 400, description = "Validation error", body = SipHooksErrorResponse),
            (status = 405, description = "Host defined in application config cannot be modified", body = SipHooksErrorResponse),
            (status = 500, description = "Failed to write hooks cache", body = SipHooksErrorResponse)
        ),
        tag = "sip"
    )
)]
pub async fn delete_sip_hooks(
    State(state): State<Arc<AppState>>,
    Json(request): Json<DeleteSipHooksRequest>,
) -> Result<Json<SipHooksResponse>, (StatusCode, Json<SipHooksErrorResponse>)> {
    // Validate request has at least one host
    if request.hosts.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(SipHooksErrorResponse {
                error: "No hosts provided to delete.".to_string(),
            }),
        ));
    }

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

    let protected_hosts = configured_sip_hosts(&state);

    if let Some(error) = reject_protected_hosts(
        &protected_hosts,
        request.hosts.iter().map(|host| host.as_str()),
    ) {
        return Err(error);
    }

    // Read existing hooks from cache
    let existing_hooks = sip_hooks::read_hooks_cache(&cache_path)
        .await
        .unwrap_or_default();

    let filtered_existing_hooks: Vec<CachedSipHook> = existing_hooks
        .into_iter()
        .filter(|hook| !protected_hosts.contains(&hook.host))
        .collect();

    // Remove the specified hosts from cache
    let remaining_hooks =
        sip_hooks::remove_hooks_by_hosts(&filtered_existing_hooks, &request.hosts);

    // Write remaining hooks back to cache
    sip_hooks::write_hooks_cache(&cache_path, &remaining_hooks)
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

    // Update runtime state and get the fully merged hooks (config + cache)
    // This will automatically revert to config values for any deleted hosts
    let response_hooks: Vec<SipHookEntry> =
        if let Some(hooks_state) = state.core_state.get_sip_hooks_state() {
            let mut hooks_guard = hooks_state.write().await;
            let original_hooks = state
                .config
                .sip
                .as_ref()
                .map(|c| c.hooks.as_slice())
                .unwrap_or(&[]);
            // update_hooks merges cache with config - config hooks reappear if removed from cache
            let full_state = hooks_guard.update_hooks(remaining_hooks.clone(), original_hooks);

            info!(
                hook_count = full_state.len(),
                deleted_hosts = ?request.hosts,
                "Deleted SIP hooks from cache and updated runtime state"
            );

            full_state
                .into_iter()
                .map(|h| SipHookEntry {
                    host: h.host,
                    url: h.url,
                })
                .collect()
        } else {
            // No SIP state configured, return the cache-only result
            info!(
                hook_count = remaining_hooks.len(),
                deleted_hosts = ?request.hosts,
                "Deleted SIP hooks from cache (no runtime state)"
            );

            remaining_hooks
                .into_iter()
                .map(|h| SipHookEntry {
                    host: h.host,
                    url: h.url,
                })
                .collect()
        };

    Ok(Json(SipHooksResponse {
        hooks: response_hooks,
    }))
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

    #[test]
    fn test_delete_sip_hooks_request_deserialization() {
        let json = r#"{"hosts": ["example.com", "other.com"]}"#;
        let request: DeleteSipHooksRequest =
            serde_json::from_str(json).expect("Failed to deserialize");

        assert_eq!(request.hosts.len(), 2);
        assert_eq!(request.hosts[0], "example.com");
        assert_eq!(request.hosts[1], "other.com");
    }

    #[test]
    fn test_delete_sip_hooks_request_empty_hosts() {
        let json = r#"{"hosts": []}"#;
        let request: DeleteSipHooksRequest =
            serde_json::from_str(json).expect("Failed to deserialize");
        assert!(request.hosts.is_empty());
    }

    #[test]
    fn test_delete_sip_hooks_request_default() {
        let json = r#"{}"#;
        let request: DeleteSipHooksRequest =
            serde_json::from_str(json).expect("Failed to deserialize");
        assert!(request.hosts.is_empty());
    }

    #[test]
    fn test_reject_protected_hosts_blocks_configured_host() {
        let protected_hosts: HashSet<String> = ["example.com".to_string()].into_iter().collect();

        let result = reject_protected_hosts(&protected_hosts, ["example.com"].iter().copied());
        assert!(result.is_some());
        let (status, Json(error)) = result.unwrap();
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert!(
            error.error.contains("example.com"),
            "error message should mention blocked host"
        );
    }

    #[test]
    fn test_reject_protected_hosts_allows_runtime_hosts() {
        let protected_hosts: HashSet<String> = ["example.com".to_string()].into_iter().collect();

        let result = reject_protected_hosts(&protected_hosts, ["other.com"].iter().copied());
        assert!(result.is_none());
    }
}
