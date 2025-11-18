use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::Json,
};
use bytes::Bytes;
use livekit_api::access_token::TokenVerifier;
use livekit_api::webhooks::{WebhookError, WebhookReceiver};
use livekit_protocol::WebhookEvent;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, info, warn};

use crate::AppState;

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
    let livekit_api_key = state
        .config
        .livekit_api_key
        .as_ref()
        .ok_or_else(|| {
            error!("LiveKit API key not configured, cannot validate webhook");
            StatusCode::SERVICE_UNAVAILABLE
        })?;

    let livekit_api_secret = state
        .config
        .livekit_api_secret
        .as_ref()
        .ok_or_else(|| {
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
    let auth_token = auth_token
        .strip_prefix("Bearer ")
        .unwrap_or(auth_token);

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
        match e {
            WebhookError::InvalidAuth(_) |
            WebhookError::InvalidSignature => {
                warn!("Invalid webhook signature or auth: {}", e);
                StatusCode::UNAUTHORIZED
            }
            WebhookError::InvalidData(_) => {
                warn!("Invalid JSON in webhook body: {}", e);
                StatusCode::BAD_REQUEST
            }
            WebhookError::InvalidBase64(_) => {
                warn!("Invalid base64 in webhook signature: {}", e);
                StatusCode::UNAUTHORIZED
            }
        }
    })?;

    // Step 5: Log the event with structured fields
    log_webhook_event(&event);

    // Step 6: Respond quickly with success
    Ok(Json(json!({
        "status": "received"
    })))
}

/// Logs webhook event details with structured fields.
///
/// Focuses on:
/// - Event ID and name
/// - Room name
/// - Participant identity, name, and kind
/// - SIP-related attributes (keys starting with "sip.")
fn log_webhook_event(event: &WebhookEvent) {
    let event_name = event.event.clone();
    let event_id = event.id.clone();
    let room_name = event.room.as_ref().map(|r| r.name.as_str());

    // Extract participant info if present
    let participant = event.participant.as_ref();
    let participant_identity = participant.map(|p| p.identity.as_str());
    let participant_name = participant.map(|p| p.name.as_str());
    let participant_kind = participant.map(|p| p.kind);

    // Extract SIP attributes (keys starting with "sip.")
    let sip_attributes: HashMap<String, String> = participant
        .map(|p| {
            p.attributes
                .iter()
                .filter(|(k, _)| k.starts_with("sip."))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
        .unwrap_or_default();

    // Log with structured fields
    info!(
        event_id = %event_id,
        event_name = %event_name,
        room_name = room_name,
        participant_identity = participant_identity,
        participant_name = participant_name,
        participant_kind = ?participant_kind,
        sip_attributes = ?sip_attributes,
        "Received LiveKit webhook event (no side effects applied yet)"
    );
}
