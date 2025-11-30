use std::env;
use std::path::PathBuf;

use super::ServerConfig;
use super::sip::{SipConfig, SipHookConfig};
use super::utils::parse_bool;
use super::yaml::YamlConfig;

/// Merge YAML configuration with environment variables
///
/// Priority order (highest to lowest):
/// 1. YAML configuration values
/// 2. Environment variables
/// 3. Default values
///
/// This allows environment variables to provide base configuration while YAML
/// can override specific values for different deployment environments.
///
/// # Arguments
/// * `yaml_config` - Optional YAML configuration to use as overrides
///
/// # Returns
/// * `Result<ServerConfig, Box<dyn std::error::Error>>` - The merged configuration or an error
pub fn merge_config(
    yaml_config: Option<YamlConfig>,
) -> Result<ServerConfig, Box<dyn std::error::Error>> {
    let yaml = yaml_config.unwrap_or_default();

    // Helper macro to get value with priority: YAML > ENV > Default
    macro_rules! get_value {
        ($env_var:expr, $yaml_value:expr, $default:expr) => {
            $yaml_value
                .or_else(|| env::var($env_var).ok())
                .unwrap_or_else(|| $default.to_string())
        };
    }

    // Helper macro for optional values: YAML > ENV
    macro_rules! get_optional {
        ($env_var:expr, $yaml_value:expr) => {
            $yaml_value.or_else(|| env::var($env_var).ok())
        };
    }

    // Server configuration
    let host = get_value!(
        "HOST",
        yaml.server.as_ref().and_then(|s| s.host.clone()),
        "0.0.0.0"
    );

    let port = if let Some(yaml_port) = yaml.server.as_ref().and_then(|s| s.port) {
        yaml_port
    } else if let Ok(port_str) = env::var("PORT") {
        port_str
            .parse::<u16>()
            .map_err(|e| format!("Invalid PORT environment variable: {e}"))?
    } else {
        3001
    };

    // LiveKit configuration
    let livekit_url = get_value!(
        "LIVEKIT_URL",
        yaml.livekit.as_ref().and_then(|l| l.url.clone()),
        "ws://localhost:7880"
    );

    let livekit_public_url = get_value!(
        "LIVEKIT_PUBLIC_URL",
        yaml.livekit.as_ref().and_then(|l| l.public_url.clone()),
        "http://localhost:7880"
    );

    let livekit_api_key = get_optional!(
        "LIVEKIT_API_KEY",
        yaml.livekit.as_ref().and_then(|l| l.api_key.clone())
    );

    let livekit_api_secret = get_optional!(
        "LIVEKIT_API_SECRET",
        yaml.livekit.as_ref().and_then(|l| l.api_secret.clone())
    );

    // Provider API keys
    let deepgram_api_key = get_optional!(
        "DEEPGRAM_API_KEY",
        yaml.providers
            .as_ref()
            .and_then(|p| p.deepgram_api_key.clone())
    );

    let elevenlabs_api_key = get_optional!(
        "ELEVENLABS_API_KEY",
        yaml.providers
            .as_ref()
            .and_then(|p| p.elevenlabs_api_key.clone())
    );

    // Google Cloud credentials (can be path, JSON content, or empty for ADC)
    let google_credentials = get_optional!(
        "GOOGLE_APPLICATION_CREDENTIALS",
        yaml.providers
            .as_ref()
            .and_then(|p| p.google_credentials.clone())
    );

    // Azure Speech Services configuration
    let azure_speech_subscription_key = get_optional!(
        "AZURE_SPEECH_SUBSCRIPTION_KEY",
        yaml.providers
            .as_ref()
            .and_then(|p| p.azure_speech_subscription_key.clone())
    );

    let azure_speech_region = get_optional!(
        "AZURE_SPEECH_REGION",
        yaml.providers
            .as_ref()
            .and_then(|p| p.azure_speech_region.clone())
    );

    // Cartesia STT API key
    let cartesia_api_key = get_optional!(
        "CARTESIA_API_KEY",
        yaml.providers
            .as_ref()
            .and_then(|p| p.cartesia_api_key.clone())
    );

    // Recording S3 configuration
    let recording_s3_bucket = get_optional!(
        "RECORDING_S3_BUCKET",
        yaml.recording.as_ref().and_then(|r| r.s3_bucket.clone())
    );

    let recording_s3_region = get_optional!(
        "RECORDING_S3_REGION",
        yaml.recording.as_ref().and_then(|r| r.s3_region.clone())
    );

    let recording_s3_endpoint = get_optional!(
        "RECORDING_S3_ENDPOINT",
        yaml.recording.as_ref().and_then(|r| r.s3_endpoint.clone())
    );

    let recording_s3_access_key = get_optional!(
        "RECORDING_S3_ACCESS_KEY",
        yaml.recording
            .as_ref()
            .and_then(|r| r.s3_access_key.clone())
    );

    let recording_s3_secret_key = get_optional!(
        "RECORDING_S3_SECRET_KEY",
        yaml.recording
            .as_ref()
            .and_then(|r| r.s3_secret_key.clone())
    );

    let recording_s3_prefix = get_optional!(
        "RECORDING_S3_PREFIX",
        yaml.recording.as_ref().and_then(|r| r.s3_prefix.clone())
    );

    // Cache configuration
    let cache_path = yaml
        .cache
        .as_ref()
        .and_then(|c| c.path.clone())
        .or_else(|| env::var("CACHE_PATH").ok())
        .map(PathBuf::from);

    let cache_ttl_seconds = yaml
        .cache
        .as_ref()
        .and_then(|c| c.ttl_seconds)
        .or_else(|| {
            env::var("CACHE_TTL_SECONDS")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
        })
        .or(Some(30 * 24 * 60 * 60)); // Default to 30 days

    // Authentication configuration
    let auth_service_url = get_optional!(
        "AUTH_SERVICE_URL",
        yaml.auth.as_ref().and_then(|a| a.service_url.clone())
    );

    let auth_signing_key_path = yaml
        .auth
        .as_ref()
        .and_then(|a| a.signing_key_path.clone())
        .or_else(|| env::var("AUTH_SIGNING_KEY_PATH").ok())
        .map(PathBuf::from);

    let auth_api_secret = get_optional!(
        "AUTH_API_SECRET",
        yaml.auth.as_ref().and_then(|a| a.api_secret.clone())
    );

    let auth_timeout_seconds = yaml
        .auth
        .as_ref()
        .and_then(|a| a.timeout_seconds)
        .or_else(|| {
            env::var("AUTH_TIMEOUT_SECONDS")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
        })
        .unwrap_or(5);

    let auth_required = yaml
        .auth
        .as_ref()
        .and_then(|a| a.required)
        .or_else(|| env::var("AUTH_REQUIRED").ok().and_then(|s| parse_bool(&s)))
        .unwrap_or(false);

    // SIP configuration (merge YAML and ENV)
    let sip = merge_sip_config(yaml.sip.as_ref())?;

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
        auth_api_secret,
        auth_timeout_seconds,
        auth_required,
        sip,
    })
}

/// Merge SIP configuration from YAML and environment variables
///
/// Priority: YAML > ENV
/// For hook secrets: per-hook secret > global hook_secret (YAML > ENV)
fn merge_sip_config(
    yaml_sip: Option<&super::yaml::SipYaml>,
) -> Result<Option<SipConfig>, Box<dyn std::error::Error>> {
    // Check if any SIP env vars are set
    let env_room_prefix = env::var("SIP_ROOM_PREFIX").ok();
    let env_allowed_addresses = env::var("SIP_ALLOWED_ADDRESSES").ok();
    let env_hooks_json = env::var("SIP_HOOKS_JSON").ok();
    let env_hook_secret = env::var("SIP_HOOK_SECRET").ok();

    let has_env_sip = env_room_prefix.is_some()
        || env_allowed_addresses.is_some()
        || env_hooks_json.is_some()
        || env_hook_secret.is_some();

    // If no YAML and no ENV, return None
    if yaml_sip.is_none() && !has_env_sip {
        return Ok(None);
    }

    // Merge room_prefix (YAML > ENV)
    let room_prefix = yaml_sip
        .and_then(|s| s.room_prefix.clone())
        .or(env_room_prefix)
        .ok_or("SIP room_prefix is required when SIP configuration is present")?;

    // Merge allowed_addresses (YAML > ENV)
    let allowed_addresses = if let Some(yaml_sip) = yaml_sip
        && !yaml_sip.allowed_addresses.is_empty()
    {
        // Use YAML addresses if present
        yaml_sip.allowed_addresses.clone()
    } else if let Some(addresses_str) = env_allowed_addresses {
        // Parse from ENV (comma-separated)
        addresses_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        vec![]
    };

    // Merge hooks (YAML > ENV)
    let hooks = if let Some(yaml_sip) = yaml_sip
        && !yaml_sip.hooks.is_empty()
    {
        // Convert from YAML hooks if present
        yaml_sip
            .hooks
            .iter()
            .map(|h| SipHookConfig {
                host: h.host.clone(),
                url: h.url.clone(),
                secret: h.secret.clone(),
            })
            .collect()
    } else if let Some(hooks_json) = env_hooks_json {
        // Parse from ENV JSON
        parse_sip_hooks_json(&hooks_json)?
    } else {
        vec![]
    };

    // Merge hook_secret (YAML > ENV)
    let hook_secret = yaml_sip
        .and_then(|s| s.hook_secret.clone())
        .or(env_hook_secret);

    Ok(Some(SipConfig::new(
        room_prefix,
        allowed_addresses,
        hooks,
        hook_secret,
    )))
}

/// Parse SIP hooks from JSON string
fn parse_sip_hooks_json(json_str: &str) -> Result<Vec<SipHookConfig>, Box<dyn std::error::Error>> {
    #[derive(serde::Deserialize)]
    struct HookJson {
        host: String,
        url: String,
        #[serde(default)]
        secret: Option<String>,
    }

    let hooks: Vec<HookJson> = serde_json::from_str(json_str)
        .map_err(|e| format!("Invalid SIP_HOOKS_JSON format: {e}"))?;

    Ok(hooks
        .into_iter()
        .map(|h| SipHookConfig {
            host: h.host,
            url: h.url,
            secret: h.secret,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::super::yaml::SipHookYaml;
    use super::*;
    use serial_test::serial;
    use std::fs;
    use tempfile::TempDir;

    // Helper to clean up environment variables
    fn cleanup_env_vars() {
        unsafe {
            env::remove_var("HOST");
            env::remove_var("PORT");
            env::remove_var("LIVEKIT_URL");
            env::remove_var("LIVEKIT_PUBLIC_URL");
            env::remove_var("DEEPGRAM_API_KEY");
            env::remove_var("ELEVENLABS_API_KEY");
            env::remove_var("CACHE_PATH");
            env::remove_var("CACHE_TTL_SECONDS");
            env::remove_var("AUTH_REQUIRED");
            env::remove_var("AUTH_SERVICE_URL");
            env::remove_var("AUTH_SIGNING_KEY_PATH");
            env::remove_var("AUTH_API_SECRET");
            env::remove_var("AUTH_TIMEOUT_SECONDS");
            env::remove_var("SIP_ROOM_PREFIX");
            env::remove_var("SIP_ALLOWED_ADDRESSES");
            env::remove_var("SIP_HOOKS_JSON");
            env::remove_var("SIP_HOOK_SECRET");
            env::remove_var("RECORDING_S3_PREFIX");
        }
    }

    #[test]
    #[serial]
    fn test_merge_yaml_only() {
        cleanup_env_vars();

        let yaml = YamlConfig {
            server: Some(super::super::yaml::ServerYaml {
                host: Some("127.0.0.1".to_string()),
                port: Some(8080),
            }),
            cache: Some(super::super::yaml::CacheYaml {
                path: Some("/tmp/cache".to_string()),
                ttl_seconds: Some(3600),
            }),
            ..Default::default()
        };

        let config = merge_config(Some(yaml)).unwrap();

        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 8080);
        assert_eq!(config.cache_path, Some(PathBuf::from("/tmp/cache")));
        assert_eq!(config.cache_ttl_seconds, Some(3600));

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_merge_yaml_overrides_env() {
        cleanup_env_vars();

        let yaml = YamlConfig {
            server: Some(super::super::yaml::ServerYaml {
                host: Some("127.0.0.1".to_string()),
                port: Some(8080),
            }),
            ..Default::default()
        };

        unsafe {
            env::set_var("HOST", "0.0.0.0");
            env::set_var("PORT", "9000");
        }

        let config = merge_config(Some(yaml)).unwrap();

        // YAML overrides ENV
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 8080);

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_merge_defaults_when_no_yaml_or_env() {
        cleanup_env_vars();

        let config = merge_config(None).unwrap();

        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.port, 3001);
        assert_eq!(config.livekit_url, "ws://localhost:7880");
        assert!(!config.auth_required);

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_merge_partial_yaml() {
        cleanup_env_vars();

        let yaml = YamlConfig {
            server: Some(super::super::yaml::ServerYaml {
                port: Some(8080),
                ..Default::default()
            }),
            ..Default::default()
        };

        let config = merge_config(Some(yaml)).unwrap();

        assert_eq!(config.host, "0.0.0.0"); // default
        assert_eq!(config.port, 8080); // from yaml

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_merge_auth_config() {
        cleanup_env_vars();
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("key.pem");
        fs::write(&key_path, "fake key").unwrap();

        let yaml = YamlConfig {
            auth: Some(super::super::yaml::AuthYaml {
                required: Some(true),
                service_url: Some("https://auth.yaml.com".to_string()),
                signing_key_path: Some(key_path.to_string_lossy().to_string()),
                timeout_seconds: Some(10),
                ..Default::default()
            }),
            ..Default::default()
        };

        unsafe {
            env::set_var("AUTH_SERVICE_URL", "https://auth.env.com");
        }

        let config = merge_config(Some(yaml)).unwrap();

        assert!(config.auth_required);
        assert_eq!(
            config.auth_service_url,
            Some("https://auth.yaml.com".to_string())
        ); // YAML overrides ENV
        assert_eq!(config.auth_signing_key_path, Some(key_path)); // from YAML
        assert_eq!(config.auth_timeout_seconds, 10); // from YAML

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_merge_recording_prefix_yaml_overrides_env() {
        cleanup_env_vars();

        let yaml = YamlConfig {
            recording: Some(super::super::yaml::RecordingYaml {
                s3_prefix: Some("yaml-prefix".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        unsafe {
            env::set_var("RECORDING_S3_PREFIX", "env-prefix");
        }

        let config = merge_config(Some(yaml)).unwrap();

        assert_eq!(config.recording_s3_prefix, Some("yaml-prefix".to_string()));

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_merge_recording_prefix_env_only() {
        cleanup_env_vars();

        unsafe {
            env::set_var("RECORDING_S3_PREFIX", "env-only");
        }

        let config = merge_config(None).unwrap();

        assert_eq!(config.recording_s3_prefix, Some("env-only".to_string()));

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_merge_recording_prefix_none_when_unset() {
        cleanup_env_vars();

        let config = merge_config(None).unwrap();

        assert_eq!(config.recording_s3_prefix, None);

        cleanup_env_vars();
    }

    // SIP configuration merge tests

    #[test]
    #[serial]
    fn test_merge_sip_yaml_only() {
        cleanup_env_vars();

        let yaml = YamlConfig {
            sip: Some(super::super::yaml::SipYaml {
                room_prefix: Some("sip-".to_string()),
                allowed_addresses: vec!["192.168.1.0/24".to_string()],
                hooks: vec![SipHookYaml {
                    host: "example.com".to_string(),
                    url: "https://webhook.example.com/events".to_string(),
                    secret: None,
                }],
                hook_secret: Some("global-secret".to_string()),
            }),
            ..Default::default()
        };

        let config = merge_config(Some(yaml)).unwrap();
        let sip = config.sip.expect("SIP config should be present");

        assert_eq!(sip.room_prefix, "sip-");
        assert_eq!(sip.allowed_addresses.len(), 1);
        assert_eq!(sip.allowed_addresses[0], "192.168.1.0/24");
        assert_eq!(sip.hooks.len(), 1);
        assert_eq!(sip.hooks[0].host, "example.com");
        assert_eq!(sip.hook_secret, Some("global-secret".to_string()));

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_merge_sip_yaml_overrides_env() {
        cleanup_env_vars();

        let yaml = YamlConfig {
            sip: Some(super::super::yaml::SipYaml {
                room_prefix: Some("yaml-prefix-".to_string()),
                allowed_addresses: vec!["192.168.1.0/24".to_string()],
                hooks: vec![],
                hook_secret: Some("yaml-secret".to_string()),
            }),
            ..Default::default()
        };

        unsafe {
            env::set_var("SIP_ROOM_PREFIX", "env-prefix-");
            env::set_var("SIP_ALLOWED_ADDRESSES", "10.0.0.1, 10.0.0.2");
            env::set_var("SIP_HOOK_SECRET", "env-secret");
        }

        let config = merge_config(Some(yaml)).unwrap();
        let sip = config.sip.expect("SIP config should be present");

        assert_eq!(sip.room_prefix, "yaml-prefix-"); // YAML overrides ENV
        assert_eq!(sip.allowed_addresses.len(), 1); // YAML overrides ENV
        assert_eq!(sip.allowed_addresses[0], "192.168.1.0/24");
        assert_eq!(sip.hook_secret, Some("yaml-secret".to_string())); // YAML overrides ENV

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_merge_sip_env_only() {
        cleanup_env_vars();

        unsafe {
            env::set_var("SIP_ROOM_PREFIX", "sip-");
            env::set_var("SIP_ALLOWED_ADDRESSES", "192.168.1.0/24");
            env::set_var(
                "SIP_HOOKS_JSON",
                r#"[{"host": "example.com", "url": "https://webhook.example.com/events"}]"#,
            );
        }

        let config = merge_config(None).unwrap();
        let sip = config.sip.expect("SIP config should be present");

        assert_eq!(sip.room_prefix, "sip-");
        assert_eq!(sip.allowed_addresses.len(), 1);
        assert_eq!(sip.hooks.len(), 1);

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_merge_sip_no_config() {
        cleanup_env_vars();

        let config = merge_config(None).unwrap();
        assert!(config.sip.is_none());

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_merge_sip_partial_yaml_with_env() {
        cleanup_env_vars();

        let yaml = YamlConfig {
            sip: Some(super::super::yaml::SipYaml {
                room_prefix: Some("sip-".to_string()),
                allowed_addresses: vec![],
                hooks: vec![],
                hook_secret: None,
            }),
            ..Default::default()
        };

        unsafe {
            env::set_var("SIP_ALLOWED_ADDRESSES", "10.0.0.1");
            env::set_var("SIP_HOOK_SECRET", "env-secret");
        }

        let config = merge_config(Some(yaml)).unwrap();
        let sip = config.sip.expect("SIP config should be present");

        assert_eq!(sip.room_prefix, "sip-"); // from YAML
        // YAML has empty array, so ENV is used as fallback
        assert_eq!(sip.allowed_addresses.len(), 1); // from ENV (YAML is empty)
        assert_eq!(sip.allowed_addresses[0], "10.0.0.1");
        assert_eq!(sip.hook_secret, Some("env-secret".to_string())); // from ENV (YAML is None)

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_merge_sip_per_hook_secret_precedence() {
        cleanup_env_vars();

        let yaml = YamlConfig {
            sip: Some(super::super::yaml::SipYaml {
                room_prefix: Some("sip-".to_string()),
                allowed_addresses: vec!["192.168.1.0/24".to_string()],
                hooks: vec![
                    SipHookYaml {
                        host: "example.com".to_string(),
                        url: "https://webhook.example.com/events".to_string(),
                        secret: None, // uses global
                    },
                    SipHookYaml {
                        host: "override.com".to_string(),
                        url: "https://webhook.override.com/events".to_string(),
                        secret: Some("per-hook-override".to_string()), // overrides global
                    },
                ],
                hook_secret: Some("global-secret".to_string()),
            }),
            ..Default::default()
        };

        let config = merge_config(Some(yaml)).unwrap();
        let sip = config.sip.expect("SIP config should be present");

        assert_eq!(sip.hook_secret, Some("global-secret".to_string()));
        assert_eq!(sip.hooks.len(), 2);
        assert_eq!(sip.hooks[0].secret, None); // will use global
        assert_eq!(sip.hooks[1].secret, Some("per-hook-override".to_string())); // overrides global

        cleanup_env_vars();
    }
}
