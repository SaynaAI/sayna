use std::collections::HashMap;
use std::time::Duration;

use http::header::{AUTHORIZATION, CONTENT_TYPE};
use livekit_api::access_token::{AccessToken, AccessTokenError, SIPGrants};
use livekit_protocol as proto;
use pbjson_types::Duration as ProtoDuration;
use prost::Message;
use reqwest::Client;

use crate::livekit::LiveKitError;

/// Options for creating an inbound SIP trunk via the raw Twirp API.
#[derive(Default, Clone)]
pub struct SIPInboundTrunkOptions {
    pub metadata: Option<String>,
    pub allowed_addresses: Option<Vec<String>>,
    pub allowed_numbers: Option<Vec<String>>,
    pub auth_username: Option<String>,
    pub auth_password: Option<String>,
    pub headers: Option<HashMap<String, String>>,
    pub headers_to_attributes: Option<HashMap<String, String>>,
    pub attributes_to_headers: Option<HashMap<String, String>>,
    pub include_headers: Option<proto::SipHeaderOptions>,
    pub max_call_duration: Option<Duration>,
    pub ringing_timeout: Option<Duration>,
    pub krisp_enabled: Option<bool>,
}

/// Minimal LiveKit SIP API client that can set fields not yet exposed by the upstream SDK.
#[derive(Clone)]
pub struct SIPApiClient {
    host: String,
    api_key: String,
    api_secret: String,
    client: Client,
}

impl SIPApiClient {
    pub fn new(
        host: impl Into<String>,
        api_key: impl Into<String>,
        api_secret: impl Into<String>,
    ) -> Self {
        let host = Self::normalize_host(host.into());
        Self {
            host,
            api_key: api_key.into(),
            api_secret: api_secret.into(),
            client: Client::new(),
        }
    }

    fn normalize_host(host: String) -> String {
        if host.starts_with("ws://") {
            host.replacen("ws://", "http://", 1)
        } else if host.starts_with("wss://") {
            host.replacen("wss://", "https://", 1)
        } else {
            host
        }
    }

    fn auth_header(&self, sip_grants: SIPGrants) -> Result<String, AccessTokenError> {
        let token = AccessToken::with_api_key(&self.api_key, &self.api_secret)
            .with_sip_grants(sip_grants)
            .to_jwt()?;
        Ok(format!("Bearer {token}"))
    }

    fn twirp_endpoint(&self, service: &str, method: &str) -> String {
        format!(
            "{}/twirp/livekit.{}/{}",
            self.host.trim_end_matches('/'),
            service,
            method
        )
    }

    /// Create an inbound SIP trunk, allowing include_headers to be set.
    pub async fn create_sip_inbound_trunk(
        &self,
        name: String,
        numbers: Vec<String>,
        options: SIPInboundTrunkOptions,
    ) -> Result<proto::SipInboundTrunkInfo, LiveKitError> {
        let url = self.twirp_endpoint("SIP", "CreateSIPInboundTrunk");

        let request = proto::CreateSipInboundTrunkRequest {
            trunk: Some(proto::SipInboundTrunkInfo {
                sip_trunk_id: Default::default(),
                name,
                metadata: options.metadata.unwrap_or_default(),
                numbers,
                allowed_addresses: options.allowed_addresses.unwrap_or_default(),
                allowed_numbers: options.allowed_numbers.unwrap_or_default(),
                auth_username: options.auth_username.unwrap_or_default(),
                auth_password: options.auth_password.unwrap_or_default(),
                headers: options.headers.unwrap_or_default(),
                headers_to_attributes: options.headers_to_attributes.unwrap_or_default(),
                attributes_to_headers: options.attributes_to_headers.unwrap_or_default(),
                include_headers: options
                    .include_headers
                    .unwrap_or(proto::SipHeaderOptions::SipNoHeaders)
                    as i32,
                ringing_timeout: options.ringing_timeout.map(|d| ProtoDuration {
                    seconds: d.as_secs() as i64,
                    nanos: d.subsec_nanos() as i32,
                }),
                max_call_duration: options.max_call_duration.map(|d| ProtoDuration {
                    seconds: d.as_secs() as i64,
                    nanos: d.subsec_nanos() as i32,
                }),
                krisp_enabled: options.krisp_enabled.unwrap_or(false),
                media_encryption: proto::SipMediaEncryption::SipMediaEncryptDisable as i32,
            }),
        };

        let auth_header = self
            .auth_header(SIPGrants {
                admin: true,
                ..Default::default()
            })
            .map_err(|e| {
                LiveKitError::ConnectionFailed(format!("Failed to create auth token: {e}"))
            })?;

        // Encode request using prost
        let mut buf = Vec::new();
        request.encode(&mut buf).map_err(|e| {
            LiveKitError::ConnectionFailed(format!("Failed to encode SIP request: {e}"))
        })?;

        let resp = self
            .client
            .post(url)
            .header(CONTENT_TYPE, "application/protobuf")
            .header(AUTHORIZATION, auth_header)
            .body(buf)
            .send()
            .await
            .map_err(|e| {
                LiveKitError::ConnectionFailed(format!("Failed to send SIP request: {e}"))
            })?;

        if resp.status().is_success() {
            let bytes = resp.bytes().await.map_err(|e| {
                LiveKitError::ConnectionFailed(format!("Failed to read SIP response: {e}"))
            })?;
            proto::SipInboundTrunkInfo::decode(bytes.as_ref()).map_err(|e| {
                LiveKitError::ConnectionFailed(format!("Failed to decode SIP response: {e}"))
            })
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Err(LiveKitError::ConnectionFailed(format!(
                "LiveKit SIP returned {status}: {body}"
            )))
        }
    }

    /// Transfer a SIP participant to a new destination.
    ///
    /// This initiates a SIP REFER to transfer the call to the specified destination.
    /// The destination should be a phone number in tel: URI format (e.g., "tel:+1234567890").
    ///
    /// # Arguments
    ///
    /// * `room_name` - Name of the room containing the SIP participant
    /// * `participant_identity` - Identity of the SIP participant to transfer
    /// * `transfer_to` - Destination to transfer to (tel: URI format)
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` on successful transfer initiation, or an error if the transfer fails.
    pub async fn transfer_sip_participant(
        &self,
        room_name: &str,
        participant_identity: &str,
        transfer_to: &str,
    ) -> Result<(), LiveKitError> {
        let url = self.twirp_endpoint("SIP", "TransferSIPParticipant");

        // Set ringing_timeout to 30 seconds - this is how long the server waits for the
        // transfer destination to answer.
        let ringing_timeout = Duration::from_secs(30);

        let request = proto::TransferSipParticipantRequest {
            room_name: room_name.to_string(),
            participant_identity: participant_identity.to_string(),
            transfer_to: transfer_to.to_string(),
            play_dialtone: false,
            headers: Default::default(),
            ringing_timeout: Some(ProtoDuration {
                seconds: ringing_timeout.as_secs() as i64,
                nanos: ringing_timeout.subsec_nanos() as i32,
            }),
        };

        // TransferSIPParticipant requires BOTH VideoGrants (with room_admin for the specific room)
        // AND SIPGrants (with call permission). This matches the Go SDK:
        // withSIPGrant{Call: true}, withVideoGrant{RoomAdmin: true, Room: in.RoomName}
        let token = AccessToken::with_api_key(&self.api_key, &self.api_secret)
            .with_grants(livekit_api::access_token::VideoGrants {
                room: room_name.to_string(),
                room_admin: true,
                ..Default::default()
            })
            .with_sip_grants(SIPGrants {
                admin: false,
                call: true,
            })
            .to_jwt()
            .map_err(|e| {
                LiveKitError::ConnectionFailed(format!("Failed to create auth token: {e}"))
            })?;
        let auth_header = format!("Bearer {token}");

        let mut buf = Vec::new();
        request.encode(&mut buf).map_err(|e| {
            LiveKitError::ConnectionFailed(format!("Failed to encode transfer request: {e}"))
        })?;

        // Use a short timeout (2 seconds) for the HTTP request.
        // Real failures (permission denied, not found, etc.) respond quickly.
        // If we timeout, the transfer was likely initiated successfully but the room
        // was cleaned up before we received the response - this is expected behavior.
        let http_timeout = Duration::from_secs(2);

        let resp = self
            .client
            .post(url)
            .header(CONTENT_TYPE, "application/protobuf")
            .header(AUTHORIZATION, auth_header)
            .timeout(http_timeout)
            .body(buf)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    LiveKitError::SIPTransferRequestTimeout
                } else {
                    LiveKitError::ConnectionFailed(format!("Failed to send transfer request: {e}"))
                }
            })?;

        if resp.status().is_success() {
            // Consume the response body to properly complete the HTTP request.
            // The official SDK always reads and decodes the response, even for Empty responses.
            // This ensures the connection is properly closed.
            let _ = resp.bytes().await;
            Ok(())
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Err(LiveKitError::ConnectionFailed(format!(
                "LiveKit SIP transfer returned {status}: {body}"
            )))
        }
    }
}
