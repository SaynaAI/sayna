use std::path::PathBuf;

use super::sip::SipConfig;

/// Validate JWT authentication configuration
///
/// Ensures that if either AUTH_SERVICE_URL or AUTH_SIGNING_KEY_PATH is provided,
/// both must be present and the key file must exist.
pub fn validate_jwt_auth(
    auth_service_url: &Option<String>,
    auth_signing_key_path: &Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    // If either is set, both must be set
    if auth_service_url.is_some() || auth_signing_key_path.is_some() {
        if auth_service_url.is_none() {
            return Err("AUTH_SERVICE_URL is required when AUTH_SIGNING_KEY_PATH is set".into());
        }
        if auth_signing_key_path.is_none() {
            return Err("AUTH_SIGNING_KEY_PATH is required when AUTH_SERVICE_URL is set".into());
        }

        // Check if the signing key file exists
        if let Some(key_path) = auth_signing_key_path
            && !key_path.exists()
        {
            return Err(format!(
                "AUTH_SIGNING_KEY_PATH file does not exist: {}",
                key_path.display()
            )
            .into());
        }
    }

    Ok(())
}

/// Validate that when auth is required, at least one auth method is configured
///
/// Checks that either JWT auth (service URL + signing key) or API secret is present
/// when authentication is required.
pub fn validate_auth_required(
    auth_required: bool,
    auth_service_url: &Option<String>,
    auth_signing_key_path: &Option<PathBuf>,
    auth_api_secret: &Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    if !auth_required {
        return Ok(());
    }

    let has_jwt_auth = auth_service_url.is_some() && auth_signing_key_path.is_some();
    let has_api_secret = auth_api_secret.is_some();

    if !has_jwt_auth && !has_api_secret {
        return Err(
            "When AUTH_REQUIRED=true, either (AUTH_SERVICE_URL + AUTH_SIGNING_KEY_PATH) or AUTH_API_SECRET must be configured".into()
        );
    }

    Ok(())
}

/// Validate SIP configuration
///
/// Validates that:
/// - room_prefix is non-empty and safe for LiveKit room names (alphanumeric, '-', '_')
/// - allowed_addresses is not empty and each entry looks like an IPv4 address or CIDR
/// - hooks list has no duplicate hosts
/// - when hooks exist, each hook has an effective secret (either per-hook or global)
/// - all secrets meet minimum length requirements and are not whitespace-only
pub fn validate_sip_config(sip: &Option<SipConfig>) -> Result<(), Box<dyn std::error::Error>> {
    let Some(config) = sip else {
        return Ok(());
    };

    // Validate room_prefix
    if config.room_prefix.is_empty() {
        return Err("SIP room_prefix cannot be empty".into());
    }

    // Check that room_prefix only contains safe characters for LiveKit room names
    let safe_chars = config
        .room_prefix
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_');
    if !safe_chars {
        return Err(
            "SIP room_prefix must contain only alphanumeric characters, '-', or '_'".into(),
        );
    }

    // Validate allowed_addresses
    if config.allowed_addresses.is_empty() {
        return Err("SIP allowed_addresses cannot be empty".into());
    }

    // Basic IPv4/CIDR validation using regex
    let ip_cidr_regex = regex::Regex::new(r"^(\d{1,3}\.){3}\d{1,3}(/\d{1,2})?$")
        .map_err(|e| format!("Failed to compile IP regex: {e}"))?;

    for addr in &config.allowed_addresses {
        if !ip_cidr_regex.is_match(addr) {
            return Err(format!(
                "SIP allowed_address '{}' does not look like a valid IPv4 address or CIDR range",
                addr
            )
            .into());
        }
    }

    // Validate hooks
    let mut seen_hosts = std::collections::HashSet::new();
    for hook in &config.hooks {
        // Check for duplicate hosts
        let host_lower = hook.host.to_lowercase();
        if !seen_hosts.insert(host_lower.clone()) {
            return Err(format!("Duplicate SIP hook host: {}", host_lower).into());
        }
    }

    // Validate hook secrets: when hooks exist, require effective secrets
    if !config.hooks.is_empty() {
        for hook in &config.hooks {
            let effective_secret = hook.secret.as_ref().or(config.hook_secret.as_ref());

            if effective_secret.is_none() {
                return Err(format!(
                    "SIP hook '{}' has no secret configured. Either set hook_secret (global) or provide a per-hook secret.",
                    hook.host
                )
                .into());
            }

            // Validate the effective secret
            if let Some(secret) = effective_secret {
                validate_hook_secret(secret, &hook.host)?;
            }
        }

        // If global secret exists, validate it
        if let Some(global_secret) = &config.hook_secret {
            validate_hook_secret(global_secret, "global")?;
        }
    }

    Ok(())
}

/// Validate a hook secret for security requirements
///
/// Ensures that:
/// - Secret is not empty or whitespace-only
/// - Secret meets minimum length requirement (16 characters)
fn validate_hook_secret(
    secret: &str,
    hook_identifier: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Check for whitespace-only
    if secret.trim().is_empty() {
        return Err(format!(
            "SIP hook secret for '{}' cannot be empty or whitespace-only",
            hook_identifier
        )
        .into());
    }

    // Check minimum length
    const MIN_SECRET_LENGTH: usize = 16;
    if secret.len() < MIN_SECRET_LENGTH {
        return Err(format!(
            "SIP hook secret for '{}' must be at least {} characters long (got {})",
            hook_identifier,
            MIN_SECRET_LENGTH,
            secret.len()
        )
        .into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::sip::SipHookConfig;

    #[test]
    fn test_validate_sip_config_none() {
        let result = validate_sip_config(&None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_sip_config_valid() {
        let config = SipConfig::new(
            "sip-".to_string(),
            vec!["192.168.1.0/24".to_string(), "10.0.0.1".to_string()],
            vec![SipHookConfig {
                host: "example.com".to_string(),
                url: "https://webhook.example.com/events".to_string(),
                secret: None,
            }],
            Some("global-secret-1234567890".to_string()),
        );

        let result = validate_sip_config(&Some(config));
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_sip_config_empty_room_prefix() {
        let config = SipConfig::new(
            "".to_string(),
            vec!["192.168.1.0/24".to_string()],
            vec![],
            None,
        );

        let result = validate_sip_config(&Some(config));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("room_prefix cannot be empty")
        );
    }

    #[test]
    fn test_validate_sip_config_invalid_room_prefix_chars() {
        let config = SipConfig::new(
            "sip@test".to_string(),
            vec!["192.168.1.0/24".to_string()],
            vec![],
            None,
        );

        let result = validate_sip_config(&Some(config));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must contain only alphanumeric")
        );
    }

    #[test]
    fn test_validate_sip_config_empty_allowed_addresses() {
        let config = SipConfig::new("sip-".to_string(), vec![], vec![], None);

        let result = validate_sip_config(&Some(config));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("allowed_addresses cannot be empty")
        );
    }

    #[test]
    fn test_validate_sip_config_invalid_ip_address() {
        let config = SipConfig::new(
            "sip-".to_string(),
            vec!["not-an-ip".to_string()],
            vec![],
            None,
        );

        let result = validate_sip_config(&Some(config));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("does not look like a valid IPv4")
        );
    }

    #[test]
    fn test_validate_sip_config_duplicate_hosts() {
        let config = SipConfig::new(
            "sip-".to_string(),
            vec!["192.168.1.0/24".to_string()],
            vec![
                SipHookConfig {
                    host: "example.com".to_string(),
                    url: "https://webhook1.example.com/events".to_string(),
                    secret: None,
                },
                SipHookConfig {
                    host: "Example.COM".to_string(), // case-insensitive duplicate
                    url: "https://webhook2.example.com/events".to_string(),
                    secret: None,
                },
            ],
            Some("global-secret-1234567890".to_string()),
        );

        let result = validate_sip_config(&Some(config));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Duplicate SIP hook host")
        );
    }

    // Note: HTTPS validation for SIP hooks is not currently implemented
    // If HTTPS validation is added in the future, uncomment this test
    // #[test]
    // fn test_validate_sip_config_non_https_url() {
    //     let config = SipConfig::new(
    //         "sip-".to_string(),
    //         vec!["192.168.1.0/24".to_string()],
    //         vec![SipHookConfig {
    //             host: "example.com".to_string(),
    //             url: "http://webhook.example.com/events".to_string(), // HTTP not HTTPS
    //         }],
    //     );
    //
    //     let result = validate_sip_config(&Some(config));
    //     assert!(result.is_err());
    //     assert!(result.unwrap_err().to_string().contains("must be HTTPS"));
    // }

    #[test]
    fn test_validate_sip_config_valid_cidr() {
        let config = SipConfig::new(
            "sip-".to_string(),
            vec!["192.168.1.0/24".to_string(), "10.0.0.0/8".to_string()],
            vec![],
            None,
        );

        let result = validate_sip_config(&Some(config));
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_sip_config_valid_alphanumeric_prefix() {
        let config = SipConfig::new(
            "sip-call_123".to_string(),
            vec!["192.168.1.0/24".to_string()],
            vec![],
            None,
        );

        let result = validate_sip_config(&Some(config));
        assert!(result.is_ok());
    }

    // Secret validation tests

    #[test]
    fn test_validate_sip_config_missing_secret() {
        let config = SipConfig::new(
            "sip-".to_string(),
            vec!["192.168.1.0/24".to_string()],
            vec![SipHookConfig {
                host: "example.com".to_string(),
                url: "https://webhook.example.com/events".to_string(),
                secret: None,
            }],
            None, // No global secret
        );

        let result = validate_sip_config(&Some(config));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("has no secret configured")
        );
    }

    #[test]
    fn test_validate_sip_config_secret_too_short() {
        let config = SipConfig::new(
            "sip-".to_string(),
            vec!["192.168.1.0/24".to_string()],
            vec![SipHookConfig {
                host: "example.com".to_string(),
                url: "https://webhook.example.com/events".to_string(),
                secret: None,
            }],
            Some("short".to_string()), // Too short
        );

        let result = validate_sip_config(&Some(config));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must be at least 16 characters")
        );
    }

    #[test]
    fn test_validate_sip_config_secret_whitespace_only() {
        let config = SipConfig::new(
            "sip-".to_string(),
            vec!["192.168.1.0/24".to_string()],
            vec![SipHookConfig {
                host: "example.com".to_string(),
                url: "https://webhook.example.com/events".to_string(),
                secret: None,
            }],
            Some("                ".to_string()), // Whitespace only
        );

        let result = validate_sip_config(&Some(config));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("cannot be empty or whitespace-only")
        );
    }

    #[test]
    fn test_validate_sip_config_per_hook_secret_override() {
        let config = SipConfig::new(
            "sip-".to_string(),
            vec!["192.168.1.0/24".to_string()],
            vec![
                SipHookConfig {
                    host: "example.com".to_string(),
                    url: "https://webhook.example.com/events".to_string(),
                    secret: None, // Uses global
                },
                SipHookConfig {
                    host: "override.com".to_string(),
                    url: "https://webhook.override.com/events".to_string(),
                    secret: Some("per-hook-secret-1234567890".to_string()), // Override
                },
            ],
            Some("global-secret-1234567890".to_string()),
        );

        let result = validate_sip_config(&Some(config));
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_sip_config_per_hook_secret_too_short() {
        let config = SipConfig::new(
            "sip-".to_string(),
            vec!["192.168.1.0/24".to_string()],
            vec![SipHookConfig {
                host: "example.com".to_string(),
                url: "https://webhook.example.com/events".to_string(),
                secret: Some("short".to_string()), // Too short
            }],
            Some("global-secret-1234567890".to_string()),
        );

        let result = validate_sip_config(&Some(config));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must be at least 16 characters")
        );
    }
}
