use std::env;
use std::path::PathBuf;

use super::ServerConfig;
use super::sip::{SipConfig, SipHookConfig};
use super::utils::parse_bool;
use super::yaml::YamlConfig;

/// Merge YAML configuration with environment variables
///
/// Priority order (highest to lowest):
/// 1. Environment variables
/// 2. YAML configuration values
/// 3. Default values
///
/// This allows YAML to provide base configuration while environment variables
/// can override specific values for different deployment environments.
///
/// # Arguments
/// * `yaml_config` - Optional YAML configuration to use as base
///
/// # Returns
/// * `Result<ServerConfig, Box<dyn std::error::Error>>` - The merged configuration or an error
pub fn merge_config(
    yaml_config: Option<YamlConfig>,
) -> Result<ServerConfig, Box<dyn std::error::Error>> {
    let yaml = yaml_config.unwrap_or_default();

    // Helper macro to get value with priority: ENV > YAML > Default
    macro_rules! get_value {
        ($env_var:expr, $yaml_value:expr, $default:expr) => {
            env::var($env_var)
                .ok()
                .or($yaml_value)
                .unwrap_or_else(|| $default.to_string())
        };
    }

    // Helper macro for optional values: ENV > YAML
    macro_rules! get_optional {
        ($env_var:expr, $yaml_value:expr) => {
            env::var($env_var).ok().or($yaml_value)
        };
    }

    // Server configuration
    let host = get_value!(
        "HOST",
        yaml.server.as_ref().and_then(|s| s.host.clone()),
        "0.0.0.0"
    );

    let port = if let Ok(port_str) = env::var("PORT") {
        port_str
            .parse::<u16>()
            .map_err(|e| format!("Invalid PORT environment variable: {e}"))?
    } else {
        yaml.server.as_ref().and_then(|s| s.port).unwrap_or(3001)
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

    // Cache configuration
    let cache_path = env::var("CACHE_PATH")
        .ok()
        .or_else(|| yaml.cache.as_ref().and_then(|c| c.path.clone()))
        .map(PathBuf::from);

    let cache_ttl_seconds = if let Ok(ttl_str) = env::var("CACHE_TTL_SECONDS") {
        ttl_str.parse::<u64>().ok()
    } else {
        yaml.cache.as_ref().and_then(|c| c.ttl_seconds)
    }
    .or(Some(30 * 24 * 60 * 60)); // Default to 30 days

    // Authentication configuration
    let auth_service_url = get_optional!(
        "AUTH_SERVICE_URL",
        yaml.auth.as_ref().and_then(|a| a.service_url.clone())
    );

    let auth_signing_key_path = env::var("AUTH_SIGNING_KEY_PATH")
        .ok()
        .or_else(|| yaml.auth.as_ref().and_then(|a| a.signing_key_path.clone()))
        .map(PathBuf::from);

    let auth_api_secret = get_optional!(
        "AUTH_API_SECRET",
        yaml.auth.as_ref().and_then(|a| a.api_secret.clone())
    );

    let auth_timeout_seconds = if let Ok(timeout_str) = env::var("AUTH_TIMEOUT_SECONDS") {
        timeout_str.parse::<u64>().ok()
    } else {
        yaml.auth.as_ref().and_then(|a| a.timeout_seconds)
    }
    .unwrap_or(5);

    let auth_required = if let Ok(required_str) = env::var("AUTH_REQUIRED") {
        parse_bool(&required_str)
    } else {
        yaml.auth.as_ref().and_then(|a| a.required)
    }
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
        recording_s3_bucket,
        recording_s3_region,
        recording_s3_endpoint,
        recording_s3_access_key,
        recording_s3_secret_key,
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
/// Priority: ENV > YAML
fn merge_sip_config(
    yaml_sip: Option<&super::yaml::SipYaml>,
) -> Result<Option<SipConfig>, Box<dyn std::error::Error>> {
    // Check if any SIP env vars are set
    let env_room_prefix = env::var("SIP_ROOM_PREFIX").ok();
    let env_allowed_addresses = env::var("SIP_ALLOWED_ADDRESSES").ok();
    let env_hooks_json = env::var("SIP_HOOKS_JSON").ok();

    let has_env_sip =
        env_room_prefix.is_some() || env_allowed_addresses.is_some() || env_hooks_json.is_some();

    // If no YAML and no ENV, return None
    if yaml_sip.is_none() && !has_env_sip {
        return Ok(None);
    }

    // Merge room_prefix (ENV > YAML)
    let room_prefix = env_room_prefix
        .or_else(|| yaml_sip.and_then(|s| s.room_prefix.clone()))
        .ok_or("SIP room_prefix is required when SIP configuration is present")?;

    // Merge allowed_addresses (ENV > YAML)
    let allowed_addresses = if let Some(addresses_str) = env_allowed_addresses {
        // Parse from ENV (comma-separated)
        addresses_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        // Use YAML addresses
        yaml_sip
            .map(|s| s.allowed_addresses.clone())
            .unwrap_or_default()
    };

    // Merge hooks (ENV > YAML)
    let hooks = if let Some(hooks_json) = env_hooks_json {
        // Parse from ENV JSON
        parse_sip_hooks_json(&hooks_json)?
    } else {
        // Convert from YAML hooks
        yaml_sip
            .map(|s| {
                s.hooks
                    .iter()
                    .map(|h| SipHookConfig {
                        host: h.host.clone(),
                        url: h.url.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    Ok(Some(SipConfig::new(room_prefix, allowed_addresses, hooks)))
}

/// Parse SIP hooks from JSON string
fn parse_sip_hooks_json(json_str: &str) -> Result<Vec<SipHookConfig>, Box<dyn std::error::Error>> {
    #[derive(serde::Deserialize)]
    struct HookJson {
        host: String,
        url: String,
    }

    let hooks: Vec<HookJson> = serde_json::from_str(json_str)
        .map_err(|e| format!("Invalid SIP_HOOKS_JSON format: {e}"))?;

    Ok(hooks
        .into_iter()
        .map(|h| SipHookConfig {
            host: h.host,
            url: h.url,
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
    fn test_merge_env_overrides_yaml() {
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

        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.port, 9000);

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
            Some("https://auth.env.com".to_string())
        ); // env overrides
        assert_eq!(config.auth_signing_key_path, Some(key_path)); // from yaml
        assert_eq!(config.auth_timeout_seconds, 10); // from yaml

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
                }],
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

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_merge_sip_env_overrides_yaml() {
        cleanup_env_vars();

        let yaml = YamlConfig {
            sip: Some(super::super::yaml::SipYaml {
                room_prefix: Some("yaml-prefix-".to_string()),
                allowed_addresses: vec!["192.168.1.0/24".to_string()],
                hooks: vec![],
            }),
            ..Default::default()
        };

        unsafe {
            env::set_var("SIP_ROOM_PREFIX", "env-prefix-");
            env::set_var("SIP_ALLOWED_ADDRESSES", "10.0.0.1, 10.0.0.2");
        }

        let config = merge_config(Some(yaml)).unwrap();
        let sip = config.sip.expect("SIP config should be present");

        assert_eq!(sip.room_prefix, "env-prefix-"); // ENV overrides YAML
        assert_eq!(sip.allowed_addresses.len(), 2); // ENV overrides YAML
        assert_eq!(sip.allowed_addresses[0], "10.0.0.1");
        assert_eq!(sip.allowed_addresses[1], "10.0.0.2");

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
            }),
            ..Default::default()
        };

        unsafe {
            env::set_var("SIP_ALLOWED_ADDRESSES", "10.0.0.1");
        }

        let config = merge_config(Some(yaml)).unwrap();
        let sip = config.sip.expect("SIP config should be present");

        assert_eq!(sip.room_prefix, "sip-"); // from YAML
        assert_eq!(sip.allowed_addresses.len(), 1); // from ENV
        assert_eq!(sip.allowed_addresses[0], "10.0.0.1");

        cleanup_env_vars();
    }
}
