//! LiveKit integration handlers
//!
//! This module provides REST API endpoints for LiveKit-related operations:
//! - Token generation for participant authentication
//! - Room listing for tenant-isolated room management
//! - Participant management (removal/kick, mute)
//! - Webhook handling for LiveKit events
//!
//! # Endpoints
//!
//! - `POST /livekit/token` - Generate JWT tokens for LiveKit room access
//! - `GET /livekit/rooms` - List LiveKit rooms for the authenticated tenant
//! - `GET /livekit/rooms/{room_name}` - Get detailed room info with participants
//! - `DELETE /livekit/participant` - Remove a participant from a room
//! - `POST /livekit/participant/mute` - Mute/unmute a participant's track
//! - `POST /livekit/webhook` - Receive and process LiveKit webhook events

mod participants;
mod rooms;
mod token;
mod webhook;

// Re-export handlers for clean API access
pub use participants::{
    MuteParticipantRequest, MuteParticipantResponse, RemoveParticipantErrorResponse,
    RemoveParticipantRequest, RemoveParticipantResponse, mute_participant, remove_participant,
};
pub use rooms::{
    ListRoomsResponse, ParticipantInfo, RoomDetailsResponse, RoomInfo, get_room_details, list_rooms,
};
pub use token::{TokenRequest, TokenResponse, generate_token};
pub use webhook::{
    SIPHookEvent, SIPHookParticipant, SIPHookRoom, SipForwardingError, extract_sip_attributes,
    get_sip_host_header, handle_livekit_webhook, parse_sip_domain, webhook_error_to_status,
};

// Re-export utoipa-generated path types for OpenAPI spec generation
#[cfg(feature = "openapi")]
pub use participants::{__path_mute_participant, __path_remove_participant};
#[cfg(feature = "openapi")]
pub use rooms::{__path_get_room_details, __path_list_rooms};
#[cfg(feature = "openapi")]
pub use token::__path_generate_token;
