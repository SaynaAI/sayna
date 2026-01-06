use std::env;
use std::path::PathBuf;

use super::parse_auth_api_secrets_json;
use super::sip::{SipConfig, SipHookConfig};
use super::utils::parse_bool;
use super::validation::{validate_auth_api_secrets, validate_auth_required, validate_jwt_auth};
use super::{AuthApiSecret, ServerConfig};

impl ServerConfig {
    /// Load configuration from environment variables
    ///
    /// Reads configuration from environment variables, with sensible defaults.
    /// # Returns
    /// * `Result<Self, Box<dyn std::error::Error>>` - The loaded configuration or an error
    ///
    /// # Errors
    /// Returns an error if:
    /// - Required environment variables are malformed
    /// - Authentication configuration is invalid
    /// - JWT signing key file doesn't exist
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        // Server configuration
        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port = env::var("PORT")
            .unwrap_or_else(|_| "3001".to_string())
            .parse::<u16>()
            .map_err(|e| format!("Invalid port number: {e}"))?;

        // LiveKit configuration
        let livekit_url =
            env::var("LIVEKIT_URL").unwrap_or_else(|_| "ws://localhost:7880".to_string());
        let livekit_public_url =
            env::var("LIVEKIT_PUBLIC_URL").unwrap_or_else(|_| "http://localhost:7880".to_string());
        let livekit_api_key = env::var("LIVEKIT_API_KEY").ok();
        let livekit_api_secret = env::var("LIVEKIT_API_SECRET").ok();

        // Provider API keys
        let deepgram_api_key = env::var("DEEPGRAM_API_KEY").ok();
        let elevenlabs_api_key = env::var("ELEVENLABS_API_KEY").ok();
        // Google credentials can be:
        // - Path to service account JSON file (GOOGLE_APPLICATION_CREDENTIALS)
        // - Inline JSON content (for secrets management systems)
        // - Empty/None to use Application Default Credentials
        let google_credentials = env::var("GOOGLE_APPLICATION_CREDENTIALS").ok();

        // Azure Speech Services configuration
        // The subscription key is tied to a specific Azure region
        let azure_speech_subscription_key = env::var("AZURE_SPEECH_SUBSCRIPTION_KEY").ok();
        let azure_speech_region = env::var("AZURE_SPEECH_REGION").ok();

        // Cartesia API key (used for both STT and TTS)
        let cartesia_api_key = env::var("CARTESIA_API_KEY").ok();

        // LiveKit recording S3 configuration
        let recording_s3_bucket = env::var("RECORDING_S3_BUCKET").ok();
        let recording_s3_region = env::var("RECORDING_S3_REGION").ok();
        let recording_s3_endpoint = env::var("RECORDING_S3_ENDPOINT").ok();
        let recording_s3_access_key = env::var("RECORDING_S3_ACCESS_KEY").ok();
        let recording_s3_secret_key = env::var("RECORDING_S3_SECRET_KEY").ok();
        let recording_s3_prefix = env::var("RECORDING_S3_PREFIX").ok();

        // Cache configuration
        let cache_path = env::var("CACHE_PATH").ok().map(PathBuf::from);
        let cache_ttl_seconds = env::var("CACHE_TTL_SECONDS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .or(Some(30 * 24 * 60 * 60)); // 1 month (30 days) default

        // Authentication configuration
        let auth_service_url = env::var("AUTH_SERVICE_URL").ok();
        let auth_signing_key_path = env::var("AUTH_SIGNING_KEY_PATH").ok().map(PathBuf::from);
        let auth_api_secrets_json = env::var("AUTH_API_SECRETS_JSON").ok();
        let auth_api_secret = env::var("AUTH_API_SECRET").ok();
        let auth_api_secret_id = env::var("AUTH_API_SECRET_ID")
            .ok()
            .unwrap_or_else(|| "default".to_string());
        let auth_timeout_seconds = env::var("AUTH_TIMEOUT_SECONDS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(5);
        let auth_required = env::var("AUTH_REQUIRED")
            .ok()
            .and_then(|v| parse_bool(&v))
            .unwrap_or(false);

        let auth_api_secrets = if let Some(json) = auth_api_secrets_json {
            parse_auth_api_secrets_json(&json)?
        } else if let Some(secret) = auth_api_secret {
            vec![AuthApiSecret {
                id: auth_api_secret_id,
                secret,
            }]
        } else {
            Vec::new()
        };

        // Validate JWT auth configuration if partially provided
        validate_jwt_auth(&auth_service_url, &auth_signing_key_path)?;
        validate_auth_api_secrets(&auth_api_secrets)?;

        // Validate that when auth is required, at least one auth method is configured
        validate_auth_required(
            auth_required,
            &auth_service_url,
            &auth_signing_key_path,
            &auth_api_secrets,
        )?;

        // SIP configuration
        let sip = parse_sip_env()?;

        Ok(ServerConfig {
            host,
            port,
            livekit_url,
            livekit_public_url,
            livekit_api_key,
            livekit_api_secret,
            deepgram_api_key,
            elevenlabs_api_key,
            google_credentials,
            azure_speech_subscription_key,
            azure_speech_region,
            cartesia_api_key,
            recording_s3_bucket,
            recording_s3_region,
            recording_s3_endpoint,
            recording_s3_access_key,
            recording_s3_secret_key,
            recording_s3_prefix,
            cache_path,
            cache_ttl_seconds,
            auth_service_url,
            auth_signing_key_path,
            auth_api_secrets,
            auth_timeout_seconds,
            auth_required,
            sip,
        })
    }
}

/// Parse SIP configuration from environment variables
///
/// Reads SIP configuration from the following environment variables:
/// - SIP_ROOM_PREFIX: Room prefix for SIP calls
/// - SIP_ALLOWED_ADDRESSES: Comma-separated list of IP addresses/CIDRs
/// - SIP_HOOKS_JSON: JSON array of hook objects with host/url/secret fields
/// - SIP_HOOK_SECRET: Global signing secret for webhook requests
/// - SIP_OUTBOUND_ADDRESS: Target SIP server address for outbound trunks
/// - SIP_OUTBOUND_AUTH_USERNAME: Username for outbound trunk authentication
/// - SIP_OUTBOUND_AUTH_PASSWORD: Password for outbound trunk authentication
///
/// # Returns
/// * `Result<Option<SipConfig>, Box<dyn std::error::Error>>` - The SIP config or None
///
/// # Errors
/// Returns an error if:
/// - SIP_HOOKS_JSON is invalid JSON
/// - Hook objects are missing required fields
fn parse_sip_env() -> Result<Option<SipConfig>, Box<dyn std::error::Error>> {
    let room_prefix = env::var("SIP_ROOM_PREFIX").ok();
    let allowed_addresses_str = env::var("SIP_ALLOWED_ADDRESSES").ok();
    let hooks_json = env::var("SIP_HOOKS_JSON").ok();
    let hook_secret = env::var("SIP_HOOK_SECRET").ok();
    let outbound_address = env::var("SIP_OUTBOUND_ADDRESS").ok();
    let outbound_auth_username = env::var("SIP_OUTBOUND_AUTH_USERNAME").ok();
    let outbound_auth_password = env::var("SIP_OUTBOUND_AUTH_PASSWORD").ok();

    // If none of the SIP env vars are set, return None
    if room_prefix.is_none()
        && allowed_addresses_str.is_none()
        && hooks_json.is_none()
        && hook_secret.is_none()
        && outbound_address.is_none()
        && outbound_auth_username.is_none()
        && outbound_auth_password.is_none()
    {
        return Ok(None);
    }

    // Parse allowed addresses (comma-separated)
    let allowed_addresses = if let Some(addresses_str) = allowed_addresses_str {
        addresses_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        vec![]
    };

    // Parse hooks JSON
    let hooks = if let Some(json_str) = hooks_json {
        parse_sip_hooks_json(&json_str)?
    } else {
        vec![]
    };

    // Room prefix is required if any SIP config is present
    let room_prefix = room_prefix.ok_or(
        "SIP_ROOM_PREFIX is required when SIP configuration is provided via environment variables",
    )?;

    Ok(Some(SipConfig::new(
        room_prefix,
        allowed_addresses,
        hooks,
        hook_secret,
        outbound_address,
        outbound_auth_username,
        outbound_auth_password,
    )))
}

/// Parse SIP hooks from JSON string
///
/// Expected format: [{"host": "example.com", "url": "https://...", "secret": "optional", "auth_id": "tenant-id"}]
/// Note: auth_id is optional and defaults to empty string when not provided.
/// Validation enforces non-empty auth_id only when AUTH_REQUIRED=true.
fn parse_sip_hooks_json(json_str: &str) -> Result<Vec<SipHookConfig>, Box<dyn std::error::Error>> {
    #[derive(serde::Deserialize)]
    struct HookJson {
        host: String,
        url: String,
        #[serde(default)]
        secret: Option<String>,
        #[serde(default)]
        auth_id: String,
    }

    let hooks: Vec<HookJson> = serde_json::from_str(json_str)
        .map_err(|e| format!("Invalid SIP_HOOKS_JSON format: {e}"))?;

    Ok(hooks
        .into_iter()
        .map(|h| SipHookConfig {
            host: h.host,
            url: h.url,
            secret: h.secret,
            auth_id: h.auth_id,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::fs;
    use tempfile::TempDir;

    // Helper to clean up environment variables after tests
    fn cleanup_env_vars() {
        unsafe {
            env::remove_var("AUTH_REQUIRED");
            env::remove_var("AUTH_SERVICE_URL");
            env::remove_var("AUTH_SIGNING_KEY_PATH");
            env::remove_var("AUTH_API_SECRETS_JSON");
            env::remove_var("AUTH_API_SECRET");
            env::remove_var("AUTH_API_SECRET_ID");
            env::remove_var("AUTH_TIMEOUT_SECONDS");
            env::remove_var("HOST");
            env::remove_var("PORT");
            env::remove_var("SIP_ROOM_PREFIX");
            env::remove_var("SIP_ALLOWED_ADDRESSES");
            env::remove_var("SIP_HOOKS_JSON");
            env::remove_var("SIP_HOOK_SECRET");
            env::remove_var("SIP_OUTBOUND_ADDRESS");
            env::remove_var("SIP_OUTBOUND_AUTH_USERNAME");
            env::remove_var("SIP_OUTBOUND_AUTH_PASSWORD");
            env::remove_var("RECORDING_S3_PREFIX");
        }
    }

    #[test]
    #[serial]
    fn test_from_env_auth_disabled_defaults() {
        cleanup_env_vars();

        // Auth disabled by default
        let config = ServerConfig::from_env().expect("Should load config");
        assert!(!config.auth_required);
        assert_eq!(config.auth_timeout_seconds, 5);
        assert!(config.auth_service_url.is_none());
        assert!(config.auth_signing_key_path.is_none());

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_auth_required_true_variants() {
        cleanup_env_vars();
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("test_key.pem");
        fs::write(&key_path, "fake key content").unwrap();

        unsafe {
            // Test "true"
            env::set_var("AUTH_REQUIRED", "true");
            env::set_var("AUTH_SERVICE_URL", "http://auth.example.com");
            env::set_var("AUTH_SIGNING_KEY_PATH", key_path.to_str().unwrap());
        }
        let config = ServerConfig::from_env().expect("Should load config");
        assert!(config.auth_required);

        unsafe {
            // Test "1"
            env::set_var("AUTH_REQUIRED", "1");
        }
        let config = ServerConfig::from_env().expect("Should load config");
        assert!(config.auth_required);

        unsafe {
            // Test "yes"
            env::set_var("AUTH_REQUIRED", "yes");
        }
        let config = ServerConfig::from_env().expect("Should load config");
        assert!(config.auth_required);

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_auth_required_false_variants() {
        cleanup_env_vars();

        unsafe {
            // Test "false"
            env::set_var("AUTH_REQUIRED", "false");
        }
        let config = ServerConfig::from_env().expect("Should load config");
        assert!(!config.auth_required);

        unsafe {
            // Test "0"
            env::set_var("AUTH_REQUIRED", "0");
        }
        let config = ServerConfig::from_env().expect("Should load config");
        assert!(!config.auth_required);

        unsafe {
            // Test "no"
            env::set_var("AUTH_REQUIRED", "no");
        }
        let config = ServerConfig::from_env().expect("Should load config");
        assert!(!config.auth_required);

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_auth_service_url() {
        cleanup_env_vars();
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("test_key.pem");
        fs::write(&key_path, "fake key content").unwrap();

        unsafe {
            env::set_var("AUTH_REQUIRED", "true");
            env::set_var("AUTH_SERVICE_URL", "https://auth.service.example.com");
            env::set_var("AUTH_SIGNING_KEY_PATH", key_path.to_str().unwrap());
        }

        let config = ServerConfig::from_env().expect("Should load config");
        assert_eq!(
            config.auth_service_url,
            Some("https://auth.service.example.com".to_string())
        );

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_auth_signing_key_path() {
        cleanup_env_vars();
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("signing_key.pem");
        fs::write(&key_path, "fake key content").unwrap();

        unsafe {
            env::set_var("AUTH_REQUIRED", "true");
            env::set_var("AUTH_SERVICE_URL", "http://auth.example.com");
            env::set_var("AUTH_SIGNING_KEY_PATH", key_path.to_str().unwrap());
        }

        let config = ServerConfig::from_env().expect("Should load config");
        assert_eq!(config.auth_signing_key_path, Some(key_path));

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_auth_timeout_default() {
        cleanup_env_vars();

        let config = ServerConfig::from_env().expect("Should load config");
        assert_eq!(config.auth_timeout_seconds, 5);

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_auth_timeout_custom() {
        cleanup_env_vars();
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("test_key.pem");
        fs::write(&key_path, "fake key content").unwrap();

        unsafe {
            env::set_var("AUTH_REQUIRED", "true");
            env::set_var("AUTH_SERVICE_URL", "http://auth.example.com");
            env::set_var("AUTH_SIGNING_KEY_PATH", key_path.to_str().unwrap());
            env::set_var("AUTH_TIMEOUT_SECONDS", "10");
        }

        let config = ServerConfig::from_env().expect("Should load config");
        assert_eq!(config.auth_timeout_seconds, 10);

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_auth_required_missing_url() {
        cleanup_env_vars();
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("test_key.pem");
        fs::write(&key_path, "fake key content").unwrap();

        unsafe {
            env::set_var("AUTH_REQUIRED", "true");
            env::set_var("AUTH_SIGNING_KEY_PATH", key_path.to_str().unwrap());
            // Missing AUTH_SERVICE_URL (and no AUTH_API_SECRETS_JSON/AUTH_API_SECRET either)
        }

        let result = ServerConfig::from_env();
        assert!(result.is_err());
        // Should fail because when AUTH_SIGNING_KEY_PATH is set, AUTH_SERVICE_URL is required
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("AUTH_SERVICE_URL is required")
        );

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_auth_required_missing_key_path() {
        cleanup_env_vars();

        unsafe {
            env::set_var("AUTH_REQUIRED", "true");
            env::set_var("AUTH_SERVICE_URL", "http://auth.example.com");
            // Missing AUTH_SIGNING_KEY_PATH (and no AUTH_API_SECRETS_JSON/AUTH_API_SECRET either)
        }

        let result = ServerConfig::from_env();
        assert!(result.is_err());
        // Should fail because when AUTH_SERVICE_URL is set, AUTH_SIGNING_KEY_PATH is required
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("AUTH_SIGNING_KEY_PATH is required")
        );

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_auth_required_key_file_not_exists() {
        cleanup_env_vars();

        unsafe {
            env::set_var("AUTH_REQUIRED", "true");
            env::set_var("AUTH_SERVICE_URL", "http://auth.example.com");
            env::set_var("AUTH_SIGNING_KEY_PATH", "/nonexistent/key.pem");
        }

        let result = ServerConfig::from_env();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("file does not exist")
        );

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_host_and_port() {
        cleanup_env_vars();

        unsafe {
            env::set_var("HOST", "127.0.0.1");
            env::set_var("PORT", "8080");
        }

        let config = ServerConfig::from_env().expect("Should load config");
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 8080);

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_recording_s3_prefix() {
        cleanup_env_vars();

        unsafe {
            env::set_var("SIP_ROOM_PREFIX", "sip-"); // Required if SIP vars present
            env::set_var("RECORDING_S3_PREFIX", "recordings/production");
        }

        let config = ServerConfig::from_env().expect("Should load config");
        assert_eq!(
            config.recording_s3_prefix,
            Some("recordings/production".to_string())
        );

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_recording_s3_prefix_empty_string() {
        cleanup_env_vars();

        unsafe {
            env::set_var("RECORDING_S3_PREFIX", "");
        }

        let config = ServerConfig::from_env().expect("Should load config");
        assert_eq!(config.recording_s3_prefix, Some(String::new()));

        unsafe {
            env::remove_var("RECORDING_S3_PREFIX");
        }

        let config_without_prefix = ServerConfig::from_env().expect("Should load config");
        assert_eq!(config_without_prefix.recording_s3_prefix, None);

        cleanup_env_vars();
    }

    // API Secret authentication tests

    #[test]
    #[serial]
    fn test_from_env_api_secret_auth_enabled() {
        cleanup_env_vars();

        unsafe {
            env::set_var("AUTH_REQUIRED", "true");
            env::set_var("AUTH_API_SECRET", "my-super-secret-token");
        }

        let config = ServerConfig::from_env().expect("Should load config");
        assert!(config.auth_required);
        assert_eq!(config.auth_api_secrets.len(), 1);
        assert_eq!(config.auth_api_secrets[0].id, "default");
        assert_eq!(
            config.auth_api_secrets[0].secret,
            "my-super-secret-token".to_string()
        );
        assert!(config.auth_service_url.is_none());
        assert!(config.auth_signing_key_path.is_none());

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_api_secrets_json_auth_enabled() {
        cleanup_env_vars();

        unsafe {
            env::set_var("AUTH_REQUIRED", "true");
            env::set_var(
                "AUTH_API_SECRETS_JSON",
                r#"[{"id":"client-a","secret":"token-a"},{"id":"client-b","secret":"token-b"}]"#,
            );
        }

        let config = ServerConfig::from_env().expect("Should load config");
        assert_eq!(config.auth_api_secrets.len(), 2);
        assert_eq!(config.auth_api_secrets[0].id, "client-a");
        assert_eq!(config.auth_api_secrets[1].id, "client-b");

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_api_secrets_json_invalid() {
        cleanup_env_vars();

        unsafe {
            env::set_var("AUTH_API_SECRETS_JSON", "not-json");
        }

        let result = ServerConfig::from_env();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid AUTH_API_SECRETS_JSON format")
        );

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_api_secret_id_legacy() {
        cleanup_env_vars();

        unsafe {
            env::set_var("AUTH_REQUIRED", "true");
            env::set_var("AUTH_API_SECRET", "my-secret");
            env::set_var("AUTH_API_SECRET_ID", "legacy-client");
        }

        let config = ServerConfig::from_env().expect("Should load config");
        assert_eq!(config.auth_api_secrets.len(), 1);
        assert_eq!(config.auth_api_secrets[0].id, "legacy-client");
        assert_eq!(config.auth_api_secrets[0].secret, "my-secret");

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_api_secret_missing_when_required() {
        cleanup_env_vars();

        unsafe {
            env::set_var("AUTH_REQUIRED", "true");
            // No AUTH_API_SECRETS_JSON/AUTH_API_SECRET or AUTH_SERVICE_URL
        }

        let result = ServerConfig::from_env();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains(
                    "either (AUTH_SERVICE_URL + AUTH_SIGNING_KEY_PATH) or AUTH_API_SECRETS_JSON/AUTH_API_SECRET"
                )
        );

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_both_auth_methods_configured() {
        cleanup_env_vars();
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("test_key.pem");
        fs::write(&key_path, "fake key content").unwrap();

        unsafe {
            env::set_var("AUTH_REQUIRED", "true");
            env::set_var("AUTH_SERVICE_URL", "http://auth.example.com");
            env::set_var("AUTH_SIGNING_KEY_PATH", key_path.to_str().unwrap());
            env::set_var("AUTH_API_SECRET", "my-secret");
        }

        // Both auth methods configured should be valid
        let config = ServerConfig::from_env().expect("Should load config with both auth methods");
        assert!(config.auth_required);
        assert_eq!(config.auth_api_secrets.len(), 1);
        assert_eq!(config.auth_api_secrets[0].secret, "my-secret");
        assert_eq!(
            config.auth_service_url,
            Some("http://auth.example.com".to_string())
        );
        assert_eq!(config.auth_signing_key_path, Some(key_path));

        cleanup_env_vars();
    }

    // SIP configuration tests

    #[test]
    #[serial]
    fn test_from_env_sip_full_config() {
        cleanup_env_vars();

        unsafe {
            env::set_var("SIP_ROOM_PREFIX", "sip-");
            env::set_var("SIP_ALLOWED_ADDRESSES", "192.168.1.0/24, 10.0.0.1");
            env::set_var(
                "SIP_HOOKS_JSON",
                r#"[{"host": "example.com", "url": "https://webhook.example.com/events", "auth_id": "tenant-1"}]"#,
            );
        }

        let config = ServerConfig::from_env().expect("Should load config");
        let sip = config.sip.expect("SIP config should be present");

        assert_eq!(sip.room_prefix, "sip-");
        assert_eq!(sip.allowed_addresses.len(), 2);
        assert_eq!(sip.allowed_addresses[0], "192.168.1.0/24");
        assert_eq!(sip.allowed_addresses[1], "10.0.0.1");
        assert_eq!(sip.hooks.len(), 1);
        assert_eq!(sip.hooks[0].host, "example.com");
        assert_eq!(sip.hooks[0].url, "https://webhook.example.com/events");
        assert_eq!(sip.hooks[0].auth_id, "tenant-1");

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_sip_no_config() {
        cleanup_env_vars();

        let config = ServerConfig::from_env().expect("Should load config");
        assert!(config.sip.is_none());

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_sip_missing_room_prefix() {
        cleanup_env_vars();

        unsafe {
            env::set_var("SIP_ALLOWED_ADDRESSES", "192.168.1.0/24");
        }

        let result = ServerConfig::from_env();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("SIP_ROOM_PREFIX is required")
        );

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_sip_invalid_hooks_json() {
        cleanup_env_vars();

        unsafe {
            env::set_var("SIP_ROOM_PREFIX", "sip-");
            env::set_var("SIP_HOOKS_JSON", "invalid json");
        }

        let result = ServerConfig::from_env();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid SIP_HOOKS_JSON")
        );

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_sip_multiple_hooks() {
        cleanup_env_vars();

        unsafe {
            env::set_var("SIP_ROOM_PREFIX", "sip-");
            env::set_var(
                "SIP_HOOKS_JSON",
                r#"[
                    {"host": "example.com", "url": "https://webhook1.example.com/events", "auth_id": "tenant-1"},
                    {"host": "another.com", "url": "https://webhook2.example.com/events", "auth_id": "tenant-2"}
                ]"#,
            );
        }

        let config = ServerConfig::from_env().expect("Should load config");
        let sip = config.sip.expect("SIP config should be present");

        assert_eq!(sip.hooks.len(), 2);
        assert_eq!(sip.hooks[0].host, "example.com");
        assert_eq!(sip.hooks[0].auth_id, "tenant-1");
        assert_eq!(sip.hooks[1].host, "another.com");
        assert_eq!(sip.hooks[1].auth_id, "tenant-2");

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_sip_addresses_with_whitespace() {
        cleanup_env_vars();

        unsafe {
            env::set_var("SIP_ROOM_PREFIX", "sip-");
            env::set_var("SIP_ALLOWED_ADDRESSES", "  192.168.1.0/24 , 10.0.0.1  ");
        }

        let config = ServerConfig::from_env().expect("Should load config");
        let sip = config.sip.expect("SIP config should be present");

        assert_eq!(sip.allowed_addresses[0], "192.168.1.0/24");
        assert_eq!(sip.allowed_addresses[1], "10.0.0.1");

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_sip_with_global_secret() {
        cleanup_env_vars();

        unsafe {
            env::set_var("SIP_ROOM_PREFIX", "sip-");
            env::set_var("SIP_ALLOWED_ADDRESSES", "192.168.1.0/24");
            env::set_var("SIP_HOOK_SECRET", "global-secret");
        }

        let config = ServerConfig::from_env().expect("Should load config");
        let sip = config.sip.expect("SIP config should be present");

        assert_eq!(sip.hook_secret, Some("global-secret".to_string()));

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_sip_with_per_hook_secret() {
        cleanup_env_vars();

        unsafe {
            env::set_var("SIP_ROOM_PREFIX", "sip-");
            env::set_var("SIP_ALLOWED_ADDRESSES", "192.168.1.0/24");
            env::set_var("SIP_HOOK_SECRET", "global-secret");
            env::set_var(
                "SIP_HOOKS_JSON",
                r#"[
                    {"host": "example.com", "url": "https://webhook.example.com/events", "auth_id": "tenant-1"},
                    {"host": "override.com", "url": "https://webhook.override.com/events", "secret": "override-secret", "auth_id": "tenant-2"}
                ]"#,
            );
        }

        let config = ServerConfig::from_env().expect("Should load config");
        let sip = config.sip.expect("SIP config should be present");

        assert_eq!(sip.hook_secret, Some("global-secret".to_string()));
        assert_eq!(sip.hooks.len(), 2);
        assert_eq!(sip.hooks[0].secret, None);
        assert_eq!(sip.hooks[0].auth_id, "tenant-1");
        assert_eq!(sip.hooks[1].secret, Some("override-secret".to_string()));
        assert_eq!(sip.hooks[1].auth_id, "tenant-2");

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_sip_with_outbound_address() {
        cleanup_env_vars();

        unsafe {
            env::set_var("SIP_ROOM_PREFIX", "sip-");
            env::set_var("SIP_ALLOWED_ADDRESSES", "192.168.1.0/24");
            env::set_var("SIP_OUTBOUND_ADDRESS", "sip.trunk.example.com:5060");
        }

        let config = ServerConfig::from_env().expect("Should load config");
        let sip = config.sip.expect("SIP config should be present");

        assert_eq!(
            sip.outbound_address,
            Some("sip.trunk.example.com:5060".to_string())
        );

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_sip_outbound_address_only_requires_room_prefix() {
        cleanup_env_vars();

        unsafe {
            env::set_var("SIP_OUTBOUND_ADDRESS", "sip.trunk.example.com");
        }

        let result = ServerConfig::from_env();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("SIP_ROOM_PREFIX is required")
        );

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_sip_outbound_address_with_whitespace() {
        cleanup_env_vars();

        unsafe {
            env::set_var("SIP_ROOM_PREFIX", "sip-");
            env::set_var("SIP_OUTBOUND_ADDRESS", "  sip.example.com  ");
        }

        let config = ServerConfig::from_env().expect("Should load config");
        let sip = config.sip.expect("SIP config should be present");

        // Whitespace should be trimmed by SipConfig::new
        assert_eq!(sip.outbound_address, Some("sip.example.com".to_string()));

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_sip_with_outbound_auth() {
        cleanup_env_vars();

        unsafe {
            env::set_var("SIP_ROOM_PREFIX", "sip-");
            env::set_var("SIP_ALLOWED_ADDRESSES", "192.168.1.0/24");
            env::set_var("SIP_OUTBOUND_ADDRESS", "sip.trunk.example.com:5060");
            env::set_var("SIP_OUTBOUND_AUTH_USERNAME", "my-user");
            env::set_var("SIP_OUTBOUND_AUTH_PASSWORD", "my-pass");
        }

        let config = ServerConfig::from_env().expect("Should load config");
        let sip = config.sip.expect("SIP config should be present");

        assert_eq!(
            sip.outbound_address,
            Some("sip.trunk.example.com:5060".to_string())
        );
        assert_eq!(sip.outbound_auth_username, Some("my-user".to_string()));
        assert_eq!(sip.outbound_auth_password, Some("my-pass".to_string()));

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_sip_outbound_auth_trims_whitespace() {
        cleanup_env_vars();

        unsafe {
            env::set_var("SIP_ROOM_PREFIX", "sip-");
            env::set_var("SIP_OUTBOUND_AUTH_USERNAME", "  my-user  ");
            env::set_var("SIP_OUTBOUND_AUTH_PASSWORD", "  my-pass  ");
        }

        let config = ServerConfig::from_env().expect("Should load config");
        let sip = config.sip.expect("SIP config should be present");

        // Whitespace should be trimmed by SipConfig::new
        assert_eq!(sip.outbound_auth_username, Some("my-user".to_string()));
        assert_eq!(sip.outbound_auth_password, Some("my-pass".to_string()));

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_sip_outbound_auth_requires_room_prefix() {
        cleanup_env_vars();

        unsafe {
            env::set_var("SIP_OUTBOUND_AUTH_USERNAME", "my-user");
            env::set_var("SIP_OUTBOUND_AUTH_PASSWORD", "my-pass");
        }

        let result = ServerConfig::from_env();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("SIP_ROOM_PREFIX is required")
        );

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_sip_outbound_auth_defaults_to_none() {
        cleanup_env_vars();

        unsafe {
            env::set_var("SIP_ROOM_PREFIX", "sip-");
            env::set_var("SIP_ALLOWED_ADDRESSES", "192.168.1.0/24");
        }

        let config = ServerConfig::from_env().expect("Should load config");
        let sip = config.sip.expect("SIP config should be present");

        assert!(sip.outbound_auth_username.is_none());
        assert!(sip.outbound_auth_password.is_none());

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_sip_hooks_json_missing_auth_id_defaults_to_empty() {
        // When auth_id is not specified in SIP_HOOKS_JSON, it should default to empty string.
        // This allows older configs to work when AUTH_REQUIRED=false.
        cleanup_env_vars();

        unsafe {
            env::set_var("SIP_ROOM_PREFIX", "sip-");
            env::set_var("SIP_ALLOWED_ADDRESSES", "192.168.1.0/24");
            env::set_var(
                "SIP_HOOKS_JSON",
                r#"[{"host": "example.com", "url": "https://webhook.example.com/events"}]"#,
            );
        }

        let config = ServerConfig::from_env().expect("Should load config");
        let sip = config.sip.expect("SIP config should be present");

        assert_eq!(sip.hooks.len(), 1);
        assert_eq!(sip.hooks[0].host, "example.com");
        assert_eq!(sip.hooks[0].url, "https://webhook.example.com/events");
        // auth_id should default to empty string when not specified
        assert_eq!(sip.hooks[0].auth_id, "");

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_sip_hooks_json_explicit_empty_auth_id() {
        // Explicit empty auth_id should also parse correctly
        cleanup_env_vars();

        unsafe {
            env::set_var("SIP_ROOM_PREFIX", "sip-");
            env::set_var("SIP_ALLOWED_ADDRESSES", "192.168.1.0/24");
            env::set_var(
                "SIP_HOOKS_JSON",
                r#"[{"host": "example.com", "url": "https://webhook.example.com/events", "auth_id": ""}]"#,
            );
        }

        let config = ServerConfig::from_env().expect("Should load config");
        let sip = config.sip.expect("SIP config should be present");

        assert_eq!(sip.hooks.len(), 1);
        assert_eq!(sip.hooks[0].auth_id, "");

        cleanup_env_vars();
    }
}
