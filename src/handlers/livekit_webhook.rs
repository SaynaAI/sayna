use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::Json,
};
use bytes::Bytes;
use livekit_api::access_token::TokenVerifier;
use livekit_api::webhooks::{WebhookError, WebhookReceiver};
use livekit_protocol::WebhookEvent;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, error, info, warn};

use crate::AppState;
use crate::utils::req_manager::ReqManager;

/// Categorizes SIP webhook forwarding failures by severity and expected action.
///
/// This enum enables structured, actionable logging for webhook forwarding failures.
/// Each variant corresponds to a specific log level and troubleshooting action.
#[derive(Debug, thiserror::Error)]
pub enum SipForwardingError {
    /// No participant in event (e.g., room_finished) - benign, not a SIP event
    #[error("No participant in event (event_type={event_type})")]
    NoParticipant { event_type: String },

    /// Missing sip.h.to attribute - participant exists but isn't SIP, or malformed SIP metadata
    #[error("No sip.h.to attribute in participant")]
    MissingSipHeader,

    /// Failed to parse SIP domain from header value - indicates upstream SIP gateway bug
    #[error("Malformed SIP header: {header}")]
    MalformedSipHeader { header: String },

    /// No hook configured for this domain - operator needs to add webhook config
    #[error("No hook configured for domain: {domain}")]
    NoHookConfigured { domain: String },

    /// Failed to create or acquire HTTP client from ReqManager pool
    #[error("Failed to acquire HTTP client: {error}")]
    HttpClientError { error: String },

    /// HTTP request failed (network error, timeout, connection refused, etc.)
    #[error("HTTP request failed for domain {domain}: {error}")]
    HttpRequestError { domain: String, error: String },

    /// Downstream hook returned non-2xx status code
    #[error("Hook returned status {status} for domain {domain}: {body}")]
    HookFailedResponse {
        domain: String,
        status: u16,
        body: String,
    },
}

impl SipForwardingError {
    /// Logs this error with the appropriate severity level and structured context.
    ///
    /// Severity mapping:
    /// - `debug`: NoParticipant, MissingSipHeader (expected for non-SIP events)
    /// - `info`: MalformedSipHeader (upstream SIP gateway bug, not our fault)
    /// - `warn`: NoHookConfigured, HttpClientError, HttpRequestError, HookFailedResponse (operator action needed)
    pub fn log_with_context(&self, event_id: &str, room_name: Option<&str>) {
        match self {
            // Debug: Expected cases for non-SIP events
            SipForwardingError::NoParticipant { event_type } => {
                debug!(
                    event_id = %event_id,
                    event_type = %event_type,
                    room_name = ?room_name,
                    "Skipping SIP forwarding: event has no participant"
                );
            }
            SipForwardingError::MissingSipHeader => {
                debug!(
                    event_id = %event_id,
                    room_name = ?room_name,
                    "Skipping SIP forwarding: participant has no sip.h.to attribute"
                );
            }

            // Info: Upstream SIP gateway issues (malformed data from external system)
            SipForwardingError::MalformedSipHeader { header } => {
                info!(
                    event_id = %event_id,
                    room_name = ?room_name,
                    sip_header = %header,
                    "Skipping SIP forwarding: malformed sip.h.to header (check upstream SIP gateway)"
                );
            }

            // Warn: Configuration or operational issues requiring operator attention
            SipForwardingError::NoHookConfigured { domain } => {
                warn!(
                    event_id = %event_id,
                    room_name = ?room_name,
                    sip_domain = %domain,
                    "SIP forwarding failed: no webhook configured for domain (add to sip.hooks config)"
                );
            }
            SipForwardingError::HttpClientError { error } => {
                warn!(
                    event_id = %event_id,
                    room_name = ?room_name,
                    error = %error,
                    "SIP forwarding failed: HTTP client error"
                );
            }
            SipForwardingError::HttpRequestError { domain, error } => {
                warn!(
                    event_id = %event_id,
                    room_name = ?room_name,
                    sip_domain = %domain,
                    error = %error,
                    "SIP forwarding failed: HTTP request error (check network/DNS/webhook endpoint)"
                );
            }
            SipForwardingError::HookFailedResponse {
                domain,
                status,
                body,
            } => {
                warn!(
                    event_id = %event_id,
                    room_name = ?room_name,
                    sip_domain = %domain,
                    status = %status,
                    response_body = %body,
                    "SIP forwarding failed: webhook returned non-2xx status"
                );
            }
        }
    }
}

/// Handler for LiveKit webhook events.
///
/// This endpoint is called by LiveKit to deliver participant/room events.
/// It validates the webhook signature using LiveKit's official SDK and logs
/// SIP-related participant attributes for troubleshooting.
///
/// The endpoint is unauthenticated (no JWT middleware) because LiveKit
/// authenticates via signed webhook payloads using the Authorization header.
pub async fn handle_livekit_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, StatusCode> {
    // Step 1: Read LiveKit credentials from config
    let livekit_api_key = state.config.livekit_api_key.as_ref().ok_or_else(|| {
        error!("LiveKit API key not configured, cannot validate webhook");
        StatusCode::SERVICE_UNAVAILABLE
    })?;

    let livekit_api_secret = state.config.livekit_api_secret.as_ref().ok_or_else(|| {
        error!("LiveKit API secret not configured, cannot validate webhook");
        StatusCode::SERVICE_UNAVAILABLE
    })?;

    // Step 2: Extract and validate Authorization header
    let auth_token = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.trim())
        .ok_or_else(|| {
            warn!("Missing Authorization header in webhook request");
            StatusCode::UNAUTHORIZED
        })?;

    // Strip optional "Bearer " prefix
    let auth_token = auth_token.strip_prefix("Bearer ").unwrap_or(auth_token);

    if auth_token.is_empty() {
        warn!("Empty Authorization token in webhook request");
        return Err(StatusCode::UNAUTHORIZED);
    }

    // Step 3: Convert body to UTF-8 string for signature verification
    let body_str = std::str::from_utf8(&body).map_err(|e| {
        warn!("Invalid UTF-8 in webhook body: {}", e);
        StatusCode::BAD_REQUEST
    })?;

    // Step 4: Verify signature and parse webhook event
    let token_verifier = TokenVerifier::with_api_key(livekit_api_key, livekit_api_secret);
    let receiver = WebhookReceiver::new(token_verifier);

    let event = receiver.receive(body_str, auth_token).map_err(|e| {
        warn!("Webhook verification failed: {}", e);
        webhook_error_to_status(&e)
    })?;

    // Step 5: Log the event with structured fields and forward to SIP hooks
    log_webhook_event(&event);

    // Step 6: Forward event to SIP-specific webhook if applicable (non-blocking)
    // Short-circuit: Only spawn forwarding task if SIP config exists and has hooks.
    // This ensures pure non-SIP deployments don't incur unnecessary background tasks
    // or noisy debug logs about missing configuration.
    if let Some(sip_config) = &state.config.sip {
        if !sip_config.hooks.is_empty() {
            // We spawn this in the background to avoid delaying the response to LiveKit
            let state_clone = state.clone();
            let body_str_owned = body_str.to_string();
            let event_clone = event.clone();
            let sip_config_clone = sip_config.clone();
            tokio::spawn(async move {
                if let Err(e) = forward_to_sip_hook(
                    &state_clone,
                    &sip_config_clone,
                    &event_clone,
                    &body_str_owned,
                )
                .await
                {
                    // Log with appropriate severity based on error type
                    let room_name = event_clone.room.as_ref().map(|r| r.name.as_str());
                    e.log_with_context(&event_clone.id, room_name);
                }
            });
        }
    }

    // Step 7: Respond quickly with success
    Ok(Json(json!({
        "status": "received"
    })))
}

/// Extracts SIP-related attributes from participant metadata.
///
/// Returns all participant attributes with keys starting with "sip.",
/// such as "sip.trunkPhoneNumber" or "sip.fromHeader".
/// Non-SIP attributes are ignored.
///
/// # Examples
///
/// ```
/// use std::collections::HashMap;
/// use livekit_protocol::ParticipantInfo;
/// use sayna::handlers::livekit_webhook::extract_sip_attributes;
///
/// let mut participant = ParticipantInfo::default();
/// participant.attributes.insert("sip.trunkPhoneNumber".to_string(), "+1234567890".to_string());
/// participant.attributes.insert("sip.fromHeader".to_string(), "User <sip:user@example.com>".to_string());
/// participant.attributes.insert("other.attribute".to_string(), "ignored".to_string());
///
/// let sip_attrs = extract_sip_attributes(&participant);
/// assert_eq!(sip_attrs.len(), 2);
/// assert_eq!(sip_attrs.get("sip.trunkPhoneNumber"), Some(&"+1234567890".to_string()));
/// assert_eq!(sip_attrs.get("sip.fromHeader"), Some(&"User <sip:user@example.com>".to_string()));
/// ```
pub fn extract_sip_attributes(
    participant: &livekit_protocol::ParticipantInfo,
) -> HashMap<String, String> {
    participant
        .attributes
        .iter()
        .filter(|(k, _)| k.starts_with("sip."))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Maps WebhookError to appropriate HTTP status code.
///
/// Returns:
/// - 401 Unauthorized for auth/signature failures (4xx tells LiveKit not to retry)
/// - 400 Bad Request for invalid data/base64 (4xx tells LiveKit not to retry)
///
/// This mapping ensures LiveKit's retry behavior works as expected:
/// - 2xx: Success, no retry
/// - 4xx: Client error, no retry
/// - 5xx: Server error, LiveKit will retry
pub fn webhook_error_to_status(error: &WebhookError) -> StatusCode {
    match error {
        WebhookError::InvalidAuth(_) | WebhookError::InvalidSignature => StatusCode::UNAUTHORIZED,
        WebhookError::InvalidData(_) => StatusCode::BAD_REQUEST,
        WebhookError::InvalidBase64(_) => StatusCode::UNAUTHORIZED,
    }
}

/// Parses a SIP domain from a SIP header value.
///
/// Handles common SIP header formats:
/// - `sip:user@domain`
/// - `"Display Name" <sip:user@domain>`
/// - `sip:user@domain;tag=xyz;user=phone`
///
/// Returns the domain in lowercase for case-insensitive matching.
/// Strips URI parameters (everything after ';') before extracting the domain.
///
/// # Examples
///
/// ```
/// use sayna::handlers::livekit_webhook::parse_sip_domain;
///
/// assert_eq!(parse_sip_domain("sip:user@example.com"), Some("example.com".to_string()));
/// assert_eq!(parse_sip_domain("\"User\" <sip:user@example.com>"), Some("example.com".to_string()));
/// assert_eq!(parse_sip_domain("sip:user@example.com;user=phone"), Some("example.com".to_string()));
/// assert_eq!(parse_sip_domain("invalid"), None);
/// ```
pub fn parse_sip_domain(header: &str) -> Option<String> {
    // Extract the SIP URI from angle brackets if present
    let uri = if let Some(start) = header.find('<') {
        if let Some(end) = header.find('>') {
            &header[start + 1..end]
        } else {
            header
        }
    } else {
        header
    };

    // Strip URI parameters (everything after ';')
    let uri = uri.split(';').next().unwrap_or(uri);

    // Extract domain from sip:user@domain format
    if let Some(sip_part) = uri.strip_prefix("sip:").or_else(|| uri.strip_prefix("sips:"))
        && let Some(at_pos) = sip_part.find('@')
    {
        let domain = &sip_part[at_pos + 1..];
        return Some(domain.trim().to_lowercase());
    }

    None
}

/// Forwards a LiveKit webhook event to a SIP-specific downstream webhook.
///
/// Extracts the SIP domain from the participant's `sip.h.to` attribute,
/// looks up the corresponding hook configuration, and forwards the event.
///
/// Uses ReqManager for connection pooling and concurrency control.
/// Does not block the main webhook handler - errors are logged but not propagated.
///
/// # Arguments
/// * `state` - Application state containing ReqManager instances
/// * `sip_config` - SIP configuration (must contain hooks). Caller ensures this is non-empty.
/// * `event` - The LiveKit webhook event to forward
/// * `body_json` - The original JSON body from LiveKit (forwarded as-is)
///
/// # Returns
/// * `Ok(())` if the event was successfully forwarded
/// * `Err(SipForwardingError)` with structured error context for logging
///
/// # Panics
/// This function assumes `sip_config` is provided and non-empty by the caller.
/// It should never be called without first checking that SIP hooks exist.
async fn forward_to_sip_hook(
    state: &Arc<AppState>,
    sip_config: &crate::config::SipConfig,
    event: &WebhookEvent,
    body_json: &str,
) -> Result<(), SipForwardingError> {
    // Step 1: Extract participant from event
    let participant = event.participant.as_ref().ok_or_else(|| {
        SipForwardingError::NoParticipant {
            event_type: event.event.clone(),
        }
    })?;

    // Step 2: Extract sip.h.to attribute
    let sip_to_header = participant
        .attributes
        .get("sip.h.to")
        .ok_or(SipForwardingError::MissingSipHeader)?;

    // Step 3: Parse domain from SIP header
    let domain = parse_sip_domain(sip_to_header).ok_or_else(|| {
        SipForwardingError::MalformedSipHeader {
            header: sip_to_header.clone(),
        }
    })?;

    // Step 4: Look up hook configuration (case-insensitive)
    let hook = sip_config
        .hooks
        .iter()
        .find(|h| h.host.eq_ignore_ascii_case(&domain))
        .ok_or_else(|| SipForwardingError::NoHookConfigured {
            domain: domain.clone(),
        })?;

    // Step 5: Get or create ReqManager for this hook host
    let req_manager =
        get_or_create_hook_manager(state, &hook.url)
            .await
            .map_err(|e| SipForwardingError::HttpClientError { error: e })?;

    // Step 6: Forward the webhook payload
    let start = Instant::now();
    let guard = req_manager.acquire().await.map_err(|e| {
        SipForwardingError::HttpClientError {
            error: e.to_string(),
        }
    })?;

    let response = guard
        .client()
        .post(&hook.url)
        .header("Content-Type", "application/json")
        .body(body_json.to_string())
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| SipForwardingError::HttpRequestError {
            domain: domain.clone(),
            error: e.to_string(),
        })?;

    let status = response.status();
    let elapsed = start.elapsed();

    if status.is_success() {
        info!(
            event_id = %event.id,
            sip_domain = %domain,
            hook_url = %hook.url,
            status = %status,
            duration_ms = elapsed.as_millis(),
            "Successfully forwarded webhook to SIP hook"
        );
        Ok(())
    } else {
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<unable to read body>".to_string());
        let truncated_body = if body.len() > 200 {
            format!("{}...", &body[..200])
        } else {
            body
        };

        Err(SipForwardingError::HookFailedResponse {
            domain,
            status: status.as_u16(),
            body: truncated_body,
        })
    }
}

/// Gets or creates a ReqManager instance for a specific hook URL.
///
/// Uses the existing CoreState infrastructure to manage per-domain connection pools.
/// Creates a new ReqManager if one doesn't exist for the given host.
///
/// # Arguments
/// * `state` - Application state
/// * `url` - The webhook URL (used to extract host for cache key)
///
/// # Returns
/// * `Ok(Arc<ReqManager>)` - A request manager for this host
/// * `Err(String)` - If ReqManager creation fails
async fn get_or_create_hook_manager(
    state: &Arc<AppState>,
    url: &str,
) -> Result<Arc<ReqManager>, String> {
    // Extract host from URL for use as cache key
    let host = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .and_then(|s| s.split('/').next())
        .ok_or_else(|| format!("Invalid webhook URL: {}", url))?;

    // Use the webhook host as the provider key (e.g., "webhook.example.com")
    let provider_key = format!("sip-hook-{}", host);

    // Try to get existing manager
    {
        let managers = state.core_state.tts_req_managers.read().await;
        if let Some(manager) = managers.get(&provider_key) {
            return Ok(manager.clone());
        }
    }

    // Create new manager if it doesn't exist
    let mut managers = state.core_state.tts_req_managers.write().await;

    // Double-check in case another task created it while we were waiting for the write lock
    if let Some(manager) = managers.get(&provider_key) {
        return Ok(manager.clone());
    }

    // Create new ReqManager with conservative settings for webhook forwarding
    let manager = ReqManager::new(3)
        .await
        .map_err(|e| format!("Failed to create ReqManager: {}", e))?;

    let manager = Arc::new(manager);
    managers.insert(provider_key.clone(), manager.clone());

    info!(
        provider_key = %provider_key,
        url = %url,
        "Created new ReqManager for SIP webhook forwarding"
    );

    Ok(manager)
}

/// Logs webhook event details with structured fields.
///
/// Focuses on:
/// - Event ID and name
/// - Room name
/// - Participant identity, name, and kind
/// - SIP-related attributes (keys starting with "sip.")
/// - Parsed SIP domain (if sip.h.to is present)
fn log_webhook_event(event: &WebhookEvent) {
    let event_name = event.event.clone();
    let event_id = event.id.clone();
    let room_name = event.room.as_ref().map(|r| r.name.as_str());

    // Extract participant info if present
    let participant = event.participant.as_ref();
    let participant_identity = participant.map(|p| p.identity.as_str());
    let participant_name = participant.map(|p| p.name.as_str());
    let participant_kind = participant.map(|p| p.kind);

    // Extract SIP attributes using helper
    let sip_attributes: HashMap<String, String> =
        participant.map(extract_sip_attributes).unwrap_or_default();

    // Extract and parse SIP domain from sip.h.to attribute
    let sip_domain = participant
        .and_then(|p| p.attributes.get("sip.h.to"))
        .and_then(|header| parse_sip_domain(header));

    // Log with structured fields
    info!(
        event_id = %event_id,
        event_name = %event_name,
        room_name = room_name,
        participant_identity = participant_identity,
        participant_name = participant_name,
        participant_kind = ?participant_kind,
        sip_domain = ?sip_domain,
        sip_attributes = ?sip_attributes,
        "Received LiveKit webhook event"
    );
}
