//! LiveKit webhook handler
//!
//! This module handles incoming LiveKit webhook events, validates their signatures,
//! and forwards SIP-related events to configured downstream webhooks.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::Json,
};
use bytes::Bytes;
use hmac::{Hmac, Mac};
use livekit_api::access_token::TokenVerifier;
use livekit_api::webhooks::{WebhookError, WebhookReceiver};
use livekit_protocol::WebhookEvent;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::AppState;
use crate::livekit::LiveKitError;
use crate::livekit::room_handler::LiveKitRoomHandler;
use crate::state::SipHooksState;
use crate::utils::req_manager::ReqManager;

type HmacSha256 = Hmac<Sha256>;

/// Participant information for SIP webhook events.
///
/// Contains minimal participant metadata forwarded to downstream webhooks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SIPHookParticipant {
    /// Participant's display name
    pub name: String,
    /// Participant's unique identity
    pub identity: String,
    /// Participant's session ID
    pub sid: String,
}

/// Room information for SIP webhook events.
///
/// Contains minimal room metadata forwarded to downstream webhooks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SIPHookRoom {
    /// Room name
    pub name: String,
    /// Room session ID
    pub sid: String,
}

/// SIP webhook event payload.
///
/// This is the filtered, minimal payload sent to downstream SIP webhooks
/// for `participant_joined` events. Contains only essential information
/// needed for SIP call handling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SIPHookEvent {
    /// Participant information (name, identity, sid)
    pub participant: SIPHookParticipant,
    /// Room information (name, sid)
    pub room: SIPHookRoom,
    /// Phone number that initiated the call (from sip.phoneNumber, e.g., "+1234567890")
    pub from_phone_number: String,
    /// Phone number that received the call (from sip.trunkPhoneNumber, e.g., "+0987654321")
    pub to_phone_number: String,
    /// Room prefix from SIP configuration
    pub room_prefix: String,
    /// SIP host/domain that matched the webhook configuration
    pub sip_host: String,
    /// All SIP headers from the participant's attributes (keys have `sip.h.` prefix stripped,
    /// e.g., `sip.h.to` becomes `to`, `sip.h.x-to-ip` becomes `x-to-ip`)
    pub sip_headers: HashMap<String, String>,
}

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

    /// Missing sip.phoneNumber attribute - SIP participant without caller phone number
    #[error("No sip.phoneNumber attribute in participant")]
    MissingFromPhoneNumber,

    /// Missing sip.trunkPhoneNumber attribute - SIP participant without trunk phone number
    #[error("No sip.trunkPhoneNumber attribute in participant")]
    MissingToPhoneNumber,

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

    /// Missing signing secret for webhook forwarding
    #[error("No signing secret configured for domain: {domain}")]
    MissingSigningSecret { domain: String },
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
            SipForwardingError::MissingFromPhoneNumber => {
                debug!(
                    event_id = %event_id,
                    room_name = ?room_name,
                    "Skipping SIP forwarding: participant has no sip.phoneNumber attribute"
                );
            }
            SipForwardingError::MissingToPhoneNumber => {
                debug!(
                    event_id = %event_id,
                    room_name = ?room_name,
                    "Skipping SIP forwarding: participant has no sip.trunkPhoneNumber attribute"
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
            SipForwardingError::MissingSigningSecret { domain } => {
                error!(
                    event_id = %event_id,
                    room_name = ?room_name,
                    sip_domain = %domain,
                    "SIP forwarding failed: no signing secret configured (set global hook_secret or per-hook secret)"
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

    // Step 6: Forward event to SIP-specific webhook if applicable (non-blocking)
    // Short-circuit: Only spawn forwarding task if SIP config exists and has hooks.
    // This ensures pure non-SIP deployments don't incur unnecessary background tasks
    // or noisy debug logs about missing configuration.
    if let Some(sip_hooks_state) = state.core_state.get_sip_hooks_state()
        && event.event == "participant_joined"
    {
        // Optimization: Check if the SIP domain matches a configured hook BEFORE spawning.
        // We only care about events that map to a configured SIP hook.
        let should_forward = {
            let hooks_guard = sip_hooks_state.read().await;
            if hooks_guard.get_hooks().is_empty() {
                false
            } else {
                event
                    .participant
                    .as_ref()
                    .and_then(|p| get_sip_host_header(&p.attributes))
                    .and_then(parse_sip_domain)
                    .map(|domain| hooks_guard.get_hook_for_domain(&domain).is_some())
                    .unwrap_or(false)
            }
        };

        if should_forward {
            // Step 5: Log the event with structured fields
            log_webhook_event(&event);

            // We spawn this in the background to avoid delaying the response to LiveKit
            let state_clone = state.clone();
            let event_clone = event.clone();
            let hooks_state_clone = sip_hooks_state.clone();
            // Clone the room handler if available for metadata updates
            let room_handler_clone = state.livekit_room_handler.clone();
            tokio::spawn(async move {
                if let Err(e) = forward_to_sip_hook(
                    &state_clone,
                    &hooks_state_clone,
                    &event_clone,
                    room_handler_clone.as_deref(),
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
/// use sayna::handlers::livekit::extract_sip_attributes;
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

/// Extracts the SIP host header value from participant attributes with priority.
///
/// Checks attributes in the following order:
/// 1. `sip.h.x-to-ip` - Custom header with explicit target IP/host (takes priority)
/// 2. `sip.h.to` - Standard SIP To header
///
/// This allows operators to override the routing domain via the `X-To-IP` custom
/// SIP header when the standard `To` header doesn't contain the desired target.
///
/// # Arguments
/// * `attributes` - The participant's attributes map
///
/// # Returns
/// * `Some(&str)` - The header value if found
/// * `None` - If neither attribute is present
pub fn get_sip_host_header(attributes: &HashMap<String, String>) -> Option<&str> {
    // Priority 1: Check for custom X-To-IP header
    if let Some(value) = attributes.get("sip.h.x-to-ip") {
        return Some(value.as_str());
    }
    // Priority 2: Fall back to standard To header
    attributes.get("sip.h.to").map(|s| s.as_str())
}

/// Parses a SIP domain/host from a SIP header value or plain hostname.
///
/// Handles common SIP header formats:
/// - `sip:user@domain`
/// - `"Display Name" <sip:user@domain>`
/// - `sip:user@domain;tag=xyz;user=phone`
/// - Plain hostname: `example.com` or `example.com:5060`
///
/// Returns the domain/host in lowercase for case-insensitive matching.
/// Strips URI parameters (everything after ';') before extracting the domain.
/// For plain hostnames, strips the port if present.
///
/// # Examples
///
/// ```
/// use sayna::handlers::livekit::parse_sip_domain;
///
/// // SIP URI formats
/// assert_eq!(parse_sip_domain("sip:user@example.com"), Some("example.com".to_string()));
/// assert_eq!(parse_sip_domain("\"User\" <sip:user@example.com>"), Some("example.com".to_string()));
/// assert_eq!(parse_sip_domain("sip:user@example.com;user=phone"), Some("example.com".to_string()));
///
/// // Plain hostname formats (for sip.h.x-to-ip)
/// assert_eq!(parse_sip_domain("sip-1.example.com:5060"), Some("sip-1.example.com".to_string()));
/// assert_eq!(parse_sip_domain("example.com"), Some("example.com".to_string()));
///
/// // Invalid (empty, no dot, or malformed SIP URI)
/// assert_eq!(parse_sip_domain(""), None);
/// assert_eq!(parse_sip_domain("not-a-domain"), None);
/// assert_eq!(parse_sip_domain("sip:nodomain"), None);
/// ```
pub fn parse_sip_domain(header: &str) -> Option<String> {
    let header = header.trim();
    if header.is_empty() {
        return None;
    }

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

    // Try to extract domain from sip:user@domain format
    if let Some(sip_part) = uri
        .strip_prefix("sip:")
        .or_else(|| uri.strip_prefix("sips:"))
        && let Some(at_pos) = sip_part.find('@')
    {
        let domain = &sip_part[at_pos + 1..];
        return Some(domain.trim().to_lowercase());
    }

    // Fallback: treat as plain hostname (possibly with port)
    // This handles cases like "sip-1.aivaconnect.ai:5060" from sip.h.x-to-ip
    // Only apply fallback if:
    // - Not a malformed SIP URI (doesn't start with sip:/sips:)
    // - Contains a '.' (looks like a real hostname/domain)
    // - No spaces or @ signs
    let uri = uri.trim();
    let is_sip_scheme = uri.starts_with("sip:") || uri.starts_with("sips:");
    if !uri.is_empty()
        && !is_sip_scheme
        && uri.contains('.')
        && !uri.contains(' ')
        && !uri.contains('@')
    {
        // Strip port if present (handle both hostname:port and IPv4:port)
        let host = if let Some(colon_pos) = uri.rfind(':') {
            // Check if everything after colon is digits (port number)
            let potential_port = &uri[colon_pos + 1..];
            if potential_port.chars().all(|c| c.is_ascii_digit()) && !potential_port.is_empty() {
                &uri[..colon_pos]
            } else {
                uri
            }
        } else {
            uri
        };

        if !host.is_empty() {
            return Some(host.to_lowercase());
        }
    }

    None
}

/// Generates an HMAC-SHA256 signature for webhook payload authentication.
///
/// Creates a canonical string `v1:{timestamp}:{event_id}:{payload}` and signs it
/// with HMAC-SHA256. Returns signing headers for inclusion in webhook requests.
///
/// # Arguments
/// * `secret` - The signing secret (hook-level or global)
/// * `timestamp` - Unix timestamp in seconds
/// * `event_id` - The LiveKit event ID
/// * `payload` - The JSON payload string
///
/// # Returns
/// * `Ok(headers)` - Map of signing headers (X-Sayna-Signature, X-Sayna-Timestamp, etc.)
/// * `Err(String)` - If HMAC initialization fails
///
/// # Security
/// - Uses HMAC-SHA256 for signature generation
/// - Canonical string format prevents replay attacks when combined with timestamp validation
/// - Secrets are never logged or exposed in error messages
fn generate_webhook_signature(
    secret: &str,
    timestamp: u64,
    event_id: &str,
    payload: &str,
) -> Result<HashMap<String, String>, String> {
    // Build canonical string: v1:{timestamp}:{event_id}:{payload}
    let canonical_string = format!("v1:{}:{}:{}", timestamp, event_id, payload);

    // Initialize HMAC-SHA256
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|e| format!("HMAC initialization failed: {}", e))?;

    // Compute signature
    mac.update(canonical_string.as_bytes());
    let result = mac.finalize();
    let signature_bytes = result.into_bytes();

    // Encode as hex
    let signature_hex = hex::encode(signature_bytes);

    // Build signing headers
    let mut headers = HashMap::new();
    headers.insert(
        "X-Sayna-Signature".to_string(),
        format!("v1={}", signature_hex),
    );
    headers.insert("X-Sayna-Timestamp".to_string(), timestamp.to_string());
    headers.insert("X-Sayna-Event-Id".to_string(), event_id.to_string());
    headers.insert("X-Sayna-Signature-Version".to_string(), "v1".to_string());

    Ok(headers)
}

/// Forwards a LiveKit webhook event to a SIP-specific downstream webhook.
///
/// Only forwards `participant_joined` events. Extracts the SIP domain from the
/// participant's `sip.h.to` attribute, looks up the corresponding hook configuration,
/// and forwards a filtered SIPHookEvent payload with HMAC-SHA256 signing headers.
///
/// Uses ReqManager for connection pooling and concurrency control.
/// Does not block the main webhook handler - errors are logged but not propagated.
///
/// # Room Metadata
///
/// When a `livekit_room_handler` is provided, this function also sets the
/// `room.metadata.auth_id` field using the matched hook's `auth_id` configuration.
/// This associates the room with the tenant that owns the SIP hook, enabling
/// proper tenant isolation for SIP-created rooms.
///
/// Metadata errors are logged but do not fail the webhook forwarding:
/// - If the room already has a different `auth_id`, a security warning is logged
/// - If the metadata update fails for other reasons, an error is logged
///
/// # Arguments
/// * `state` - Application state containing ReqManager instances
/// * `hooks_state` - SIP hooks runtime state (must contain hooks). Caller ensures this is non-empty.
/// * `event` - The LiveKit webhook event to forward
/// * `livekit_room_handler` - Optional room handler for setting room metadata
///
/// # Returns
/// * `Ok(())` if the event was successfully forwarded
/// * `Err(SipForwardingError)` with structured error context for logging
///
/// # Security
/// - All outbound webhooks are signed with HMAC-SHA256
/// - Requires either per-hook `secret` or global `hook_secret` in configuration
/// - Secrets are never logged; only timing and status information is recorded
/// - Room metadata auth_id conflicts are logged as security warnings
///
/// # Panics
/// This function assumes SIP hooks are configured and present in state.
/// It should never be called without first checking that SIP hooks exist.
async fn forward_to_sip_hook(
    state: &Arc<AppState>,
    hooks_state: &Arc<RwLock<SipHooksState>>,
    event: &WebhookEvent,
    livekit_room_handler: Option<&LiveKitRoomHandler>,
) -> Result<(), SipForwardingError> {
    // Step 0: Only process participant_joined events, ignore all others
    if event.event != "participant_joined" {
        return Err(SipForwardingError::NoParticipant {
            event_type: event.event.clone(),
        });
    }

    // Step 1: Extract participant from event
    let participant =
        event
            .participant
            .as_ref()
            .ok_or_else(|| SipForwardingError::NoParticipant {
                event_type: event.event.clone(),
            })?;

    // Step 2: Extract room from event
    let room = event
        .room
        .as_ref()
        .ok_or_else(|| SipForwardingError::NoParticipant {
            event_type: event.event.clone(),
        })?;

    // Step 3: Extract SIP host header (sip.h.x-to-ip takes priority over sip.h.to)
    let sip_host_header =
        get_sip_host_header(&participant.attributes).ok_or(SipForwardingError::MissingSipHeader)?;

    // Step 4: Extract caller phone number from sip.phoneNumber
    let from_phone_number = participant
        .attributes
        .get("sip.phoneNumber")
        .ok_or(SipForwardingError::MissingFromPhoneNumber)?;

    // Step 5: Extract trunk phone number from sip.trunkPhoneNumber
    let to_phone_number = participant
        .attributes
        .get("sip.trunkPhoneNumber")
        .ok_or(SipForwardingError::MissingToPhoneNumber)?;

    // Step 6: Parse domain from SIP header
    let domain = parse_sip_domain(sip_host_header).ok_or_else(|| {
        SipForwardingError::MalformedSipHeader {
            header: sip_host_header.to_string(),
        }
    })?;

    // Step 7/8: Look up hook configuration and resolve signing secret (case-insensitive)
    let (hook_url, signing_secret, room_prefix, auth_id) = {
        let hooks_guard = hooks_state.read().await;
        let hook = hooks_guard.get_hook_for_domain(&domain).ok_or_else(|| {
            SipForwardingError::NoHookConfigured {
                domain: domain.clone(),
            }
        })?;

        let signing_secret = hooks_guard.get_signing_secret(hook).ok_or_else(|| {
            SipForwardingError::MissingSigningSecret {
                domain: domain.clone(),
            }
        })?;

        (
            hook.url.clone(),
            signing_secret.to_string(),
            hooks_guard.get_room_prefix().to_string(),
            hook.auth_id.clone(),
        )
    };

    // Step 8.5: Set room metadata auth_id if LiveKit room handler is available
    // This associates the SIP-created room with the tenant that owns the hook.
    // Done before webhook forwarding so downstream sees consistent state.
    // Skip when auth_id is empty (unauthenticated mode - no tenant enforcement)
    if let Some(room_handler) = livekit_room_handler {
        if auth_id.trim().is_empty() {
            debug!(
                event_id = %event.id,
                room_name = %room.name,
                sip_domain = %domain,
                "Skipping room metadata auth_id update: auth_id is empty (unauthenticated mode)"
            );
        } else {
            let room_name = &room.name;
            match room_handler.ensure_room_auth_id(room_name, &auth_id).await {
                Ok(()) => {
                    info!(
                        event_id = %event.id,
                        room_name = %room_name,
                        auth_id = %auth_id,
                        sip_domain = %domain,
                        "Set room metadata auth_id for SIP call"
                    );
                }
                Err(LiveKitError::MetadataConflict {
                    existing,
                    attempted,
                }) => {
                    // Security warning: room already has a different auth_id
                    // This could indicate a cross-tenant attempt or misconfiguration
                    warn!(
                        event_id = %event.id,
                        room_name = %room_name,
                        existing_auth_id = %existing,
                        attempted_auth_id = %attempted,
                        sip_domain = %domain,
                        "SECURITY: Room metadata auth_id conflict - room belongs to different tenant"
                    );
                    // Continue with webhook forwarding - don't block the call flow
                }
                Err(LiveKitError::RoomNotFound(name)) => {
                    // Room doesn't exist yet or was deleted - not expected for participant_joined
                    warn!(
                        event_id = %event.id,
                        room_name = %name,
                        auth_id = %auth_id,
                        sip_domain = %domain,
                        "Room not found when setting auth_id (race condition or deleted)"
                    );
                }
                Err(e) => {
                    // Other errors (network, API errors)
                    warn!(
                        event_id = %event.id,
                        room_name = %room_name,
                        auth_id = %auth_id,
                        sip_domain = %domain,
                        error = %e,
                        "Failed to set room metadata auth_id"
                    );
                }
            }
        }
    }

    // Step 9: Build SIPHookEvent payload
    let sip_headers: HashMap<String, String> = participant
        .attributes
        .iter()
        .filter_map(|(k, v)| {
            k.strip_prefix("sip.h.")
                .map(|header_name| (header_name.to_string(), v.clone()))
        })
        .collect();

    let sip_event = SIPHookEvent {
        participant: SIPHookParticipant {
            name: participant.name.clone(),
            identity: participant.identity.clone(),
            sid: participant.sid.clone(),
        },
        room: SIPHookRoom {
            name: room.name.clone(),
            sid: room.sid.clone(),
        },
        from_phone_number: from_phone_number.clone(),
        to_phone_number: to_phone_number.clone(),
        room_prefix: room_prefix.clone(),
        sip_host: domain.clone(),
        sip_headers,
    };

    // Step 10: Serialize payload
    let payload =
        serde_json::to_string(&sip_event).map_err(|e| SipForwardingError::HttpClientError {
            error: format!("Failed to serialize SIPHookEvent: {}", e),
        })?;

    // Step 11: Generate HMAC signature and headers
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| SipForwardingError::HttpClientError {
            error: format!("System time error: {}", e),
        })?
        .as_secs();

    let signing_headers =
        generate_webhook_signature(&signing_secret, timestamp, &event.id, &payload)
            .map_err(|e| SipForwardingError::HttpClientError { error: e })?;

    // Step 12: Get or create ReqManager for this hook host
    let req_manager = get_or_create_hook_manager(state, &hook_url)
        .await
        .map_err(|e| SipForwardingError::HttpClientError { error: e })?;

    // Step 13: Forward the webhook payload with signing headers
    let start = Instant::now();
    let guard = req_manager
        .acquire()
        .await
        .map_err(|e| SipForwardingError::HttpClientError {
            error: e.to_string(),
        })?;

    let mut request = guard
        .client()
        .post(&hook_url)
        .header("Content-Type", "application/json");

    // Add signing headers
    for (key, value) in signing_headers {
        request = request.header(key, value);
    }

    let response = request
        .body(payload)
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
            hook_url = %hook_url,
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

    // Extract and parse SIP domain (sip.h.x-to-ip takes priority over sip.h.to)
    let sip_domain = participant
        .and_then(|p| get_sip_host_header(&p.attributes))
        .and_then(parse_sip_domain);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sip_hook_event_serialization() {
        // Create a SIPHookEvent
        let mut sip_headers = HashMap::new();
        sip_headers.insert("to".to_string(), "sip:user@example.com".to_string());
        sip_headers.insert("from".to_string(), "sip:caller@other.com".to_string());
        sip_headers.insert("x-to-ip".to_string(), "10.0.0.1:5060".to_string());

        let event = SIPHookEvent {
            participant: SIPHookParticipant {
                name: "SIP User".to_string(),
                identity: "sip-user-123".to_string(),
                sid: "PA_abc123".to_string(),
            },
            room: SIPHookRoom {
                name: "sip-room-456".to_string(),
                sid: "RM_xyz789".to_string(),
            },
            from_phone_number: "+1234567890".to_string(),
            to_phone_number: "+0987654321".to_string(),
            room_prefix: "sip-".to_string(),
            sip_host: "example.com".to_string(),
            sip_headers,
        };

        // Serialize to JSON
        let json = serde_json::to_string(&event).expect("Failed to serialize SIPHookEvent");

        // Parse back to verify structure
        let parsed: Value = serde_json::from_str(&json).expect("Failed to parse JSON");

        // Verify participant fields
        assert_eq!(parsed["participant"]["name"], "SIP User");
        assert_eq!(parsed["participant"]["identity"], "sip-user-123");
        assert_eq!(parsed["participant"]["sid"], "PA_abc123");

        // Verify room fields
        assert_eq!(parsed["room"]["name"], "sip-room-456");
        assert_eq!(parsed["room"]["sid"], "RM_xyz789");

        // Verify other fields
        assert_eq!(parsed["from_phone_number"], "+1234567890");
        assert_eq!(parsed["to_phone_number"], "+0987654321");
        assert_eq!(parsed["room_prefix"], "sip-");
        assert_eq!(parsed["sip_host"], "example.com");

        // Verify sip_headers
        assert_eq!(parsed["sip_headers"]["to"], "sip:user@example.com");
        assert_eq!(parsed["sip_headers"]["from"], "sip:caller@other.com");
        assert_eq!(parsed["sip_headers"]["x-to-ip"], "10.0.0.1:5060");
        assert_eq!(parsed["sip_headers"].as_object().unwrap().len(), 3);

        // Verify no extra fields (7 total: participant, room, from_phone_number, to_phone_number, room_prefix, sip_host, sip_headers)
        assert_eq!(parsed.as_object().unwrap().len(), 7);
    }

    #[test]
    fn test_sip_hook_event_deserialization() {
        // Create JSON payload
        let json = r#"{
            "participant": {
                "name": "Test Caller",
                "identity": "caller-789",
                "sid": "PA_test456"
            },
            "room": {
                "name": "test-room",
                "sid": "RM_test123"
            },
            "from_phone_number": "+9876543210",
            "to_phone_number": "+1234567890",
            "room_prefix": "call-",
            "sip_host": "test.example.org",
            "sip_headers": {
                "to": "sip:agent@test.example.org",
                "x-custom": "custom-value"
            }
        }"#;

        // Deserialize
        let event: SIPHookEvent =
            serde_json::from_str(json).expect("Failed to deserialize SIPHookEvent");

        // Verify fields
        assert_eq!(event.participant.name, "Test Caller");
        assert_eq!(event.participant.identity, "caller-789");
        assert_eq!(event.participant.sid, "PA_test456");
        assert_eq!(event.room.name, "test-room");
        assert_eq!(event.room.sid, "RM_test123");
        assert_eq!(event.from_phone_number, "+9876543210");
        assert_eq!(event.to_phone_number, "+1234567890");
        assert_eq!(event.room_prefix, "call-");
        assert_eq!(event.sip_host, "test.example.org");
        assert_eq!(event.sip_headers.len(), 2);
        assert_eq!(
            event.sip_headers.get("to").unwrap(),
            "sip:agent@test.example.org"
        );
        assert_eq!(event.sip_headers.get("x-custom").unwrap(), "custom-value");
    }

    #[test]
    fn test_sip_hook_participant_serialization() {
        let participant = SIPHookParticipant {
            name: "Alice".to_string(),
            identity: "alice-123".to_string(),
            sid: "PA_alice".to_string(),
        };

        let json = serde_json::to_string(&participant).expect("Failed to serialize");
        let parsed: Value = serde_json::from_str(&json).expect("Failed to parse");

        assert_eq!(parsed["name"], "Alice");
        assert_eq!(parsed["identity"], "alice-123");
        assert_eq!(parsed["sid"], "PA_alice");
        assert_eq!(parsed.as_object().unwrap().len(), 3);
    }

    #[test]
    fn test_sip_hook_room_serialization() {
        let room = SIPHookRoom {
            name: "conference-room".to_string(),
            sid: "RM_conf123".to_string(),
        };

        let json = serde_json::to_string(&room).expect("Failed to serialize");
        let parsed: Value = serde_json::from_str(&json).expect("Failed to parse");

        assert_eq!(parsed["name"], "conference-room");
        assert_eq!(parsed["sid"], "RM_conf123");
        assert_eq!(parsed.as_object().unwrap().len(), 2);
    }

    #[test]
    fn test_generate_webhook_signature_deterministic() {
        // Test deterministic signature generation with fixed inputs
        let secret = "test-secret-key";
        let timestamp = 1700000000u64;
        let event_id = "EVT_test123";
        let payload = r#"{"test":"data"}"#;

        let result = super::generate_webhook_signature(secret, timestamp, event_id, payload);
        assert!(result.is_ok());

        let headers = result.unwrap();

        // Verify all expected headers are present
        assert!(headers.contains_key("X-Sayna-Signature"));
        assert!(headers.contains_key("X-Sayna-Timestamp"));
        assert!(headers.contains_key("X-Sayna-Event-Id"));
        assert!(headers.contains_key("X-Sayna-Signature-Version"));

        // Verify header values
        assert_eq!(headers.get("X-Sayna-Timestamp").unwrap(), "1700000000");
        assert_eq!(headers.get("X-Sayna-Event-Id").unwrap(), "EVT_test123");
        assert_eq!(headers.get("X-Sayna-Signature-Version").unwrap(), "v1");

        // Verify signature format (should be v1=<hex>)
        let signature = headers.get("X-Sayna-Signature").unwrap();
        assert!(signature.starts_with("v1="));
        assert_eq!(signature.len(), 3 + 64); // "v1=" + 64 hex chars (SHA256)

        // Verify determinism: same inputs should produce same signature
        let result2 = super::generate_webhook_signature(secret, timestamp, event_id, payload);
        assert!(result2.is_ok());
        let headers2 = result2.unwrap();
        assert_eq!(
            headers.get("X-Sayna-Signature"),
            headers2.get("X-Sayna-Signature")
        );
    }

    #[test]
    fn test_generate_webhook_signature_different_secrets() {
        // Different secrets should produce different signatures
        let timestamp = 1700000000u64;
        let event_id = "EVT_test123";
        let payload = r#"{"test":"data"}"#;

        let headers1 =
            super::generate_webhook_signature("secret1", timestamp, event_id, payload).unwrap();
        let headers2 =
            super::generate_webhook_signature("secret2", timestamp, event_id, payload).unwrap();

        assert_ne!(
            headers1.get("X-Sayna-Signature"),
            headers2.get("X-Sayna-Signature")
        );
    }

    #[test]
    fn test_generate_webhook_signature_different_timestamps() {
        // Different timestamps should produce different signatures
        let secret = "test-secret";
        let event_id = "EVT_test123";
        let payload = r#"{"test":"data"}"#;

        let headers1 =
            super::generate_webhook_signature(secret, 1700000000, event_id, payload).unwrap();
        let headers2 =
            super::generate_webhook_signature(secret, 1700000001, event_id, payload).unwrap();

        assert_ne!(
            headers1.get("X-Sayna-Signature"),
            headers2.get("X-Sayna-Signature")
        );
        assert_eq!(headers1.get("X-Sayna-Timestamp").unwrap(), "1700000000");
        assert_eq!(headers2.get("X-Sayna-Timestamp").unwrap(), "1700000001");
    }

    #[test]
    fn test_generate_webhook_signature_different_event_ids() {
        // Different event IDs should produce different signatures
        let secret = "test-secret";
        let timestamp = 1700000000u64;
        let payload = r#"{"test":"data"}"#;

        let headers1 =
            super::generate_webhook_signature(secret, timestamp, "EVT_123", payload).unwrap();
        let headers2 =
            super::generate_webhook_signature(secret, timestamp, "EVT_456", payload).unwrap();

        assert_ne!(
            headers1.get("X-Sayna-Signature"),
            headers2.get("X-Sayna-Signature")
        );
        assert_eq!(headers1.get("X-Sayna-Event-Id").unwrap(), "EVT_123");
        assert_eq!(headers2.get("X-Sayna-Event-Id").unwrap(), "EVT_456");
    }

    #[test]
    fn test_generate_webhook_signature_different_payloads() {
        // Different payloads should produce different signatures
        let secret = "test-secret";
        let timestamp = 1700000000u64;
        let event_id = "EVT_test123";

        let headers1 =
            super::generate_webhook_signature(secret, timestamp, event_id, r#"{"a":"1"}"#).unwrap();
        let headers2 =
            super::generate_webhook_signature(secret, timestamp, event_id, r#"{"a":"2"}"#).unwrap();

        assert_ne!(
            headers1.get("X-Sayna-Signature"),
            headers2.get("X-Sayna-Signature")
        );
    }

    #[test]
    fn test_generate_webhook_signature_empty_secret() {
        // Empty secret should still work (though not recommended)
        let result =
            super::generate_webhook_signature("", 1700000000, "EVT_123", r#"{"test":"data"}"#);
        assert!(result.is_ok());
    }

    #[test]
    fn test_generate_webhook_signature_special_characters() {
        // Test with special characters in payload
        let secret = "test-secret!@#$%";
        let timestamp = 1700000000u64;
        let event_id = "EVT_test123";
        let payload = r#"{"special":"chars: 你好, émoji: 🚀"}"#;

        let result = super::generate_webhook_signature(secret, timestamp, event_id, payload);
        assert!(result.is_ok());

        let headers = result.unwrap();
        assert!(headers.get("X-Sayna-Signature").unwrap().starts_with("v1="));
    }

    #[test]
    fn test_sip_forwarding_error_missing_signing_secret() {
        let error = SipForwardingError::MissingSigningSecret {
            domain: "example.com".to_string(),
        };

        let error_msg = error.to_string();
        assert!(error_msg.contains("example.com"));
        assert!(error_msg.contains("signing secret"));
    }

    #[test]
    fn test_get_sip_host_header_x_to_ip_priority() {
        // When both sip.h.x-to-ip and sip.h.to are present, x-to-ip takes priority
        let mut attrs = HashMap::new();
        attrs.insert(
            "sip.h.x-to-ip".to_string(),
            "sip:user@priority-host.com".to_string(),
        );
        attrs.insert(
            "sip.h.to".to_string(),
            "sip:user@fallback-host.com".to_string(),
        );

        let result = super::get_sip_host_header(&attrs);
        assert_eq!(result, Some("sip:user@priority-host.com"));
    }

    #[test]
    fn test_get_sip_host_header_fallback_to_to() {
        // When only sip.h.to is present, use it as fallback
        let mut attrs = HashMap::new();
        attrs.insert(
            "sip.h.to".to_string(),
            "sip:user@fallback-host.com".to_string(),
        );

        let result = super::get_sip_host_header(&attrs);
        assert_eq!(result, Some("sip:user@fallback-host.com"));
    }

    #[test]
    fn test_get_sip_host_header_only_x_to_ip() {
        // When only sip.h.x-to-ip is present
        let mut attrs = HashMap::new();
        attrs.insert(
            "sip.h.x-to-ip".to_string(),
            "sip:user@priority-host.com".to_string(),
        );

        let result = super::get_sip_host_header(&attrs);
        assert_eq!(result, Some("sip:user@priority-host.com"));
    }

    #[test]
    fn test_get_sip_host_header_neither_present() {
        // When neither attribute is present
        let mut attrs = HashMap::new();
        attrs.insert("sip.other".to_string(), "value".to_string());

        let result = super::get_sip_host_header(&attrs);
        assert_eq!(result, None);
    }

    #[test]
    fn test_get_sip_host_header_empty_attrs() {
        // Empty attributes map
        let attrs = HashMap::new();

        let result = super::get_sip_host_header(&attrs);
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_sip_domain_plain_hostname_with_port() {
        // Plain hostname with port (typical sip.h.x-to-ip format)
        assert_eq!(
            super::parse_sip_domain("sip-1.aivaconnect.ai:5060"),
            Some("sip-1.aivaconnect.ai".to_string())
        );
    }

    #[test]
    fn test_parse_sip_domain_plain_hostname_without_port() {
        // Plain hostname without port
        assert_eq!(
            super::parse_sip_domain("example.com"),
            Some("example.com".to_string())
        );
    }

    #[test]
    fn test_parse_sip_domain_plain_ip_with_port() {
        // Plain IP address with port
        assert_eq!(
            super::parse_sip_domain("192.168.1.100:5060"),
            Some("192.168.1.100".to_string())
        );
    }

    #[test]
    fn test_parse_sip_domain_plain_hostname_case_insensitive() {
        // Plain hostname should be lowercased
        assert_eq!(
            super::parse_sip_domain("SIP-Server.EXAMPLE.COM:5060"),
            Some("sip-server.example.com".to_string())
        );
    }
}
