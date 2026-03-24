//! Shared provider authentication resolution.
//!
//! This module normalizes provider authentication from two possible sources:
//! - Session/request-scoped auth input provided by WebSocket or REST callers
//! - Server configuration defaults loaded from environment variables or YAML
//!
//! The outer provider name is always the source of truth. The auth input object
//! contains only that provider's auth fields and is validated here before being
//! normalized into a provider-specific internal enum.

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::config::ServerConfig;
use crate::core::{stt::STTConfig, tts::TTSConfig};

/// Raw provider auth input received from API callers.
///
/// The outer provider field determines how these fields are interpreted.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProviderAuthInput {
    /// Provider-specific auth fields.
    #[serde(flatten)]
    #[cfg_attr(feature = "openapi", schema(value_type = Object, additional_properties = true))]
    pub fields: HashMap<String, Value>,
}

impl ProviderAuthInput {
    /// Returns true when the auth object is `{}`.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    fn reject_unknown_keys(
        &self,
        provider: &str,
        allowed_keys: &[&str],
    ) -> Result<(), ProviderAuthError> {
        let allowed: BTreeSet<&str> = allowed_keys.iter().copied().collect();
        let mut unknown: Vec<String> = self
            .fields
            .keys()
            .filter(|key| !allowed.contains(key.as_str()))
            .cloned()
            .collect();
        unknown.sort();

        if unknown.is_empty() {
            return Ok(());
        }

        Err(ProviderAuthError::InvalidAuthInput {
            provider: provider.to_string(),
            message: format!(
                "Unsupported auth field(s) for provider {provider}: {}",
                unknown.join(", ")
            ),
        })
    }

    fn required_string(&self, provider: &str, field: &str) -> Result<String, ProviderAuthError> {
        match self.fields.get(field) {
            Some(Value::String(value)) if !value.is_empty() => Ok(value.clone()),
            Some(Value::String(_)) => Err(ProviderAuthError::InvalidAuthInput {
                provider: provider.to_string(),
                message: format!("Auth field '{field}' is required for provider {provider}"),
            }),
            Some(_) => Err(ProviderAuthError::InvalidAuthInput {
                provider: provider.to_string(),
                message: format!("Auth field '{field}' must be a string for provider {provider}"),
            }),
            None => Err(ProviderAuthError::InvalidAuthInput {
                provider: provider.to_string(),
                message: format!("Auth field '{field}' is required for provider {provider}"),
            }),
        }
    }

    fn google_credentials(&self, provider: &str) -> Result<String, ProviderAuthError> {
        match self.fields.get("credentials") {
            Some(Value::String(value)) if !value.is_empty() => Ok(value.clone()),
            Some(Value::String(_)) => Err(ProviderAuthError::InvalidAuthInput {
                provider: provider.to_string(),
                message: format!("Auth field 'credentials' is required for provider {provider}"),
            }),
            Some(Value::Object(object)) => Ok(Value::Object(object.clone()).to_string()),
            Some(_) => Err(ProviderAuthError::InvalidAuthInput {
                provider: provider.to_string(),
                message: format!(
                    "Auth field 'credentials' must be a string or JSON object for provider {provider}"
                ),
            }),
            None => Err(ProviderAuthError::InvalidAuthInput {
                provider: provider.to_string(),
                message: format!("Auth field 'credentials' is required for provider {provider}"),
            }),
        }
    }
}

/// Normalized provider authentication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedProviderAuth {
    ApiKey { api_key: String },
    Google { credentials: String },
    AzureSpeech { api_key: String, region: String },
}

impl ResolvedProviderAuth {
    /// Returns the normalized string payload used by legacy provider configs.
    pub fn api_key(&self) -> &str {
        match self {
            ResolvedProviderAuth::ApiKey { api_key } => api_key,
            ResolvedProviderAuth::Google { credentials } => credentials,
            ResolvedProviderAuth::AzureSpeech { api_key, .. } => api_key,
        }
    }

    /// Returns the Azure region when applicable.
    pub fn azure_region(&self) -> Option<&str> {
        match self {
            ResolvedProviderAuth::AzureSpeech { region, .. } => Some(region),
            _ => None,
        }
    }

    /// Returns a stable auth fingerprint safe to include in cache keys.
    pub fn cache_fingerprint(&self) -> String {
        let material = match self {
            ResolvedProviderAuth::ApiKey { api_key } => format!("api_key|{api_key}"),
            ResolvedProviderAuth::Google { credentials } => {
                format!("google|{}", canonicalize_google_credentials(credentials))
            }
            ResolvedProviderAuth::AzureSpeech { api_key, region } => {
                format!("azure|{api_key}|{region}")
            }
        };

        format!("{:x}", Sha256::digest(material.as_bytes()))
    }

    /// Applies normalized auth to an STT config.
    pub fn apply_to_stt_config(&self, config: &mut STTConfig) {
        config.api_key = self.api_key().to_string();
        config.azure_region = self.azure_region().map(str::to_string);
    }

    /// Applies normalized auth to a TTS config.
    pub fn apply_to_tts_config(&self, config: &mut TTSConfig) {
        config.api_key = self.api_key().to_string();
        config.azure_region = self.azure_region().map(str::to_string);
    }
}

/// Errors raised during provider auth resolution.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderAuthError {
    #[error("Unsupported provider: {provider}")]
    UnsupportedProvider { provider: String },

    #[error("Invalid auth input for provider {provider}: {message}")]
    InvalidAuthInput { provider: String, message: String },

    #[error("Missing server auth for provider {provider}: {message}")]
    MissingServerAuth { provider: String, message: String },
}

/// Resolves provider auth from session/request input or server defaults.
pub fn resolve_provider_auth(
    provider: &str,
    auth_input: Option<&ProviderAuthInput>,
    server_config: &ServerConfig,
) -> Result<ResolvedProviderAuth, ProviderAuthError> {
    let provider = normalize_provider(provider);

    if auth_input.is_none_or(ProviderAuthInput::is_empty) {
        return resolve_server_provider_auth(&provider, server_config);
    }

    let auth_input = auth_input.expect("checked above");

    match provider.as_str() {
        "deepgram" | "elevenlabs" | "cartesia" => {
            auth_input.reject_unknown_keys(&provider, &["api_key"])?;
            Ok(ResolvedProviderAuth::ApiKey {
                api_key: auth_input.required_string(&provider, "api_key")?,
            })
        }
        "google" => {
            auth_input.reject_unknown_keys(&provider, &["credentials"])?;
            Ok(ResolvedProviderAuth::Google {
                credentials: auth_input.google_credentials(&provider)?,
            })
        }
        "azure" => {
            auth_input.reject_unknown_keys(&provider, &["api_key", "region"])?;
            Ok(ResolvedProviderAuth::AzureSpeech {
                api_key: auth_input.required_string(&provider, "api_key")?,
                region: auth_input.required_string(&provider, "region")?,
            })
        }
        _ => Err(ProviderAuthError::UnsupportedProvider { provider }),
    }
}

fn resolve_server_provider_auth(
    provider: &str,
    server_config: &ServerConfig,
) -> Result<ResolvedProviderAuth, ProviderAuthError> {
    match provider {
        "deepgram" => server_config
            .deepgram_api_key
            .as_ref()
            .cloned()
            .map(|api_key| ResolvedProviderAuth::ApiKey { api_key })
            .ok_or_else(|| ProviderAuthError::MissingServerAuth {
                provider: provider.to_string(),
                message: "Deepgram API key not configured in server environment".to_string(),
            }),
        "elevenlabs" => server_config
            .elevenlabs_api_key
            .as_ref()
            .cloned()
            .map(|api_key| ResolvedProviderAuth::ApiKey { api_key })
            .ok_or_else(|| ProviderAuthError::MissingServerAuth {
                provider: provider.to_string(),
                message: "ElevenLabs API key not configured in server environment".to_string(),
            }),
        "cartesia" => server_config
            .cartesia_api_key
            .as_ref()
            .cloned()
            .map(|api_key| ResolvedProviderAuth::ApiKey { api_key })
            .ok_or_else(|| ProviderAuthError::MissingServerAuth {
                provider: provider.to_string(),
                message: "Cartesia API key not configured in server environment".to_string(),
            }),
        "google" => Ok(ResolvedProviderAuth::Google {
            credentials: server_config.google_credentials.clone().unwrap_or_default(),
        }),
        "azure" => {
            let api_key = server_config
                .azure_speech_subscription_key
                .as_ref()
                .cloned()
                .ok_or_else(|| ProviderAuthError::MissingServerAuth {
                    provider: provider.to_string(),
                    message: "Azure Speech subscription key not configured in server environment"
                        .to_string(),
                })?;

            let region = server_config
                .azure_speech_region
                .clone()
                .unwrap_or_else(|| "eastus".to_string());

            Ok(ResolvedProviderAuth::AzureSpeech { api_key, region })
        }
        _ => Err(ProviderAuthError::UnsupportedProvider {
            provider: provider.to_string(),
        }),
    }
}

fn normalize_provider(provider: &str) -> String {
    match provider.to_lowercase().as_str() {
        "microsoft-azure" | "azure" => "azure".to_string(),
        "deepgram" => "deepgram".to_string(),
        "elevenlabs" => "elevenlabs".to_string(),
        "google" => "google".to_string(),
        "cartesia" => "cartesia".to_string(),
        _ => provider.to_lowercase(),
    }
}

fn canonicalize_google_credentials(credentials: &str) -> String {
    match serde_json::from_str::<Value>(credentials) {
        Ok(value) => canonical_json_string(&value),
        Err(_) => credentials.to_string(),
    }
}

fn canonical_json_string(value: &Value) -> String {
    let mut output = String::new();
    write_canonical_json(value, &mut output);
    output
}

fn write_canonical_json(value: &Value, output: &mut String) {
    match value {
        Value::Null => output.push_str("null"),
        Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Value::Number(value) => output.push_str(&value.to_string()),
        Value::String(value) => output.push_str(
            &serde_json::to_string(value).expect("serializing a JSON string should not fail"),
        ),
        Value::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write_canonical_json(value, output);
            }
            output.push(']');
        }
        Value::Object(values) => {
            output.push('{');
            let mut entries: Vec<_> = values.iter().collect();
            entries.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));

            for (index, (key, value)) in entries.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(
                    &serde_json::to_string(key)
                        .expect("serializing a JSON object key should not fail"),
                );
                output.push(':');
                write_canonical_json(value, output);
            }
            output.push('}');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuthApiSecret;
    use serde_json::json;

    fn test_server_config() -> ServerConfig {
        ServerConfig {
            host: "localhost".to_string(),
            port: 3001,
            livekit_url: "ws://localhost:7880".to_string(),
            livekit_public_url: "http://localhost:7880".to_string(),
            livekit_api_key: None,
            livekit_api_secret: None,
            deepgram_api_key: Some("dg-server-key".to_string()),
            elevenlabs_api_key: Some("el-server-key".to_string()),
            google_credentials: Some("/tmp/google.json".to_string()),
            azure_speech_subscription_key: Some("azure-server-key".to_string()),
            azure_speech_region: Some("westeurope".to_string()),
            cartesia_api_key: Some("cartesia-server-key".to_string()),
            recording_s3_bucket: None,
            recording_s3_region: None,
            recording_s3_endpoint: None,
            recording_s3_access_key: None,
            recording_s3_secret_key: None,
            recording_s3_prefix: None,
            cache_path: None,
            cache_ttl_seconds: Some(3600),
            auth_service_url: None,
            auth_signing_key_path: None,
            auth_api_secrets: vec![AuthApiSecret {
                id: "default".to_string(),
                secret: "secret".to_string(),
            }],
            auth_timeout_seconds: 5,
            auth_required: false,
            sip: None,
        }
    }

    fn auth_input(value: Value) -> ProviderAuthInput {
        serde_json::from_value(value).expect("auth input should deserialize")
    }

    #[test]
    fn test_resolve_server_fallback_deepgram() {
        let config = test_server_config();
        let resolved = resolve_provider_auth("deepgram", None, &config).unwrap();
        assert_eq!(
            resolved,
            ResolvedProviderAuth::ApiKey {
                api_key: "dg-server-key".to_string()
            }
        );
    }

    #[test]
    fn test_resolve_server_fallback_google_adc_when_missing() {
        let mut config = test_server_config();
        config.google_credentials = None;

        let resolved = resolve_provider_auth("google", None, &config).unwrap();
        assert_eq!(
            resolved,
            ResolvedProviderAuth::Google {
                credentials: String::new()
            }
        );
    }

    #[test]
    fn test_resolve_server_fallback_azure_alias() {
        let config = test_server_config();
        let resolved = resolve_provider_auth("microsoft-azure", None, &config).unwrap();
        assert_eq!(
            resolved,
            ResolvedProviderAuth::AzureSpeech {
                api_key: "azure-server-key".to_string(),
                region: "westeurope".to_string(),
            }
        );
    }

    #[test]
    fn test_resolve_empty_auth_uses_server_fallback() {
        let config = test_server_config();
        let resolved =
            resolve_provider_auth("cartesia", Some(&ProviderAuthInput::default()), &config)
                .unwrap();
        assert_eq!(
            resolved,
            ResolvedProviderAuth::ApiKey {
                api_key: "cartesia-server-key".to_string()
            }
        );
    }

    #[test]
    fn test_resolve_session_override_deepgram() {
        let config = test_server_config();
        let auth = auth_input(json!({ "api_key": "dg-session-key" }));

        let resolved = resolve_provider_auth("deepgram", Some(&auth), &config).unwrap();
        assert_eq!(
            resolved,
            ResolvedProviderAuth::ApiKey {
                api_key: "dg-session-key".to_string()
            }
        );
    }

    #[test]
    fn test_resolve_session_override_google_json_object() {
        let config = test_server_config();
        let auth = auth_input(json!({
            "credentials": {
                "type": "service_account",
                "project_id": "project-123"
            }
        }));

        let resolved = resolve_provider_auth("google", Some(&auth), &config).unwrap();
        assert_eq!(
            resolved,
            ResolvedProviderAuth::Google {
                credentials: r#"{"project_id":"project-123","type":"service_account"}"#.to_string()
            }
        );
    }

    #[test]
    fn test_resolve_session_override_google_string() {
        let config = test_server_config();
        let auth = auth_input(json!({ "credentials": "/custom/path.json" }));

        let resolved = resolve_provider_auth("google", Some(&auth), &config).unwrap();
        assert_eq!(
            resolved,
            ResolvedProviderAuth::Google {
                credentials: "/custom/path.json".to_string()
            }
        );
    }

    #[test]
    fn test_resolve_session_override_google_empty_string_rejected() {
        let config = test_server_config();
        let auth = auth_input(json!({ "credentials": "" }));

        let error = resolve_provider_auth("google", Some(&auth), &config).unwrap_err();
        assert_eq!(
            error,
            ProviderAuthError::InvalidAuthInput {
                provider: "google".to_string(),
                message: "Auth field 'credentials' is required for provider google".to_string()
            }
        );
    }

    #[test]
    fn test_resolve_session_override_azure() {
        let config = test_server_config();
        let auth = auth_input(json!({
            "api_key": "azure-session-key",
            "region": "eastus2"
        }));

        let resolved = resolve_provider_auth("azure", Some(&auth), &config).unwrap();
        assert_eq!(
            resolved,
            ResolvedProviderAuth::AzureSpeech {
                api_key: "azure-session-key".to_string(),
                region: "eastus2".to_string(),
            }
        );
    }

    #[test]
    fn test_resolve_rejects_partial_auth_without_merge() {
        let config = test_server_config();
        let auth = auth_input(json!({ "api_key": "azure-session-key" }));

        let error = resolve_provider_auth("azure", Some(&auth), &config).unwrap_err();
        assert_eq!(
            error,
            ProviderAuthError::InvalidAuthInput {
                provider: "azure".to_string(),
                message: "Auth field 'region' is required for provider azure".to_string()
            }
        );
    }

    #[test]
    fn test_resolve_rejects_invalid_field_type() {
        let config = test_server_config();
        let auth = auth_input(json!({ "api_key": 123 }));

        let error = resolve_provider_auth("deepgram", Some(&auth), &config).unwrap_err();
        assert_eq!(
            error,
            ProviderAuthError::InvalidAuthInput {
                provider: "deepgram".to_string(),
                message: "Auth field 'api_key' must be a string for provider deepgram".to_string()
            }
        );
    }

    #[test]
    fn test_resolve_rejects_unknown_fields() {
        let config = test_server_config();
        let auth = auth_input(json!({
            "api_key": "dg-session-key",
            "region": "eastus"
        }));

        let error = resolve_provider_auth("deepgram", Some(&auth), &config).unwrap_err();
        assert_eq!(
            error,
            ProviderAuthError::InvalidAuthInput {
                provider: "deepgram".to_string(),
                message: "Unsupported auth field(s) for provider deepgram: region".to_string()
            }
        );
    }

    #[test]
    fn test_resolve_missing_server_auth_is_clear() {
        let mut config = test_server_config();
        config.elevenlabs_api_key = None;

        let error = resolve_provider_auth("elevenlabs", None, &config).unwrap_err();
        assert_eq!(
            error,
            ProviderAuthError::MissingServerAuth {
                provider: "elevenlabs".to_string(),
                message: "ElevenLabs API key not configured in server environment".to_string()
            }
        );
    }

    #[test]
    fn test_resolve_unsupported_provider() {
        let config = test_server_config();
        let error = resolve_provider_auth("unsupported", None, &config).unwrap_err();
        assert_eq!(
            error,
            ProviderAuthError::UnsupportedProvider {
                provider: "unsupported".to_string()
            }
        );
    }

    #[test]
    fn test_apply_resolved_auth_to_configs() {
        let resolved = ResolvedProviderAuth::AzureSpeech {
            api_key: "azure-session-key".to_string(),
            region: "eastus2".to_string(),
        };
        let mut stt_config = STTConfig::default();
        let mut tts_config = TTSConfig::default();

        resolved.apply_to_stt_config(&mut stt_config);
        resolved.apply_to_tts_config(&mut tts_config);

        assert_eq!(stt_config.api_key, "azure-session-key");
        assert_eq!(stt_config.azure_region.as_deref(), Some("eastus2"));
        assert_eq!(tts_config.api_key, "azure-session-key");
        assert_eq!(tts_config.azure_region.as_deref(), Some("eastus2"));
    }

    #[test]
    fn test_cache_fingerprint_stable_for_google_json_key_order() {
        let config = test_server_config();
        let auth_one = auth_input(json!({
            "credentials": {
                "type": "service_account",
                "project_id": "project-123"
            }
        }));
        let auth_two = auth_input(json!({
            "credentials": {
                "project_id": "project-123",
                "type": "service_account"
            }
        }));

        let resolved_one = resolve_provider_auth("google", Some(&auth_one), &config).unwrap();
        let resolved_two = resolve_provider_auth("google", Some(&auth_two), &config).unwrap();

        assert_eq!(
            resolved_one.cache_fingerprint(),
            resolved_two.cache_fingerprint()
        );
    }

    #[test]
    fn test_cache_fingerprint_differs_for_distinct_credentials() {
        let left = ResolvedProviderAuth::ApiKey {
            api_key: "key-one".to_string(),
        };
        let right = ResolvedProviderAuth::ApiKey {
            api_key: "key-two".to_string(),
        };

        assert_ne!(left.cache_fingerprint(), right.cache_fingerprint());
    }
}
