use std::path::PathBuf;

use super::AuthApiSecret;
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
    auth_api_secrets: &[AuthApiSecret],
) -> Result<(), Box<dyn std::error::Error>> {
    if !auth_required {
        return Ok(());
    }

    let has_jwt_auth = auth_service_url.is_some() && auth_signing_key_path.is_some();
    let has_api_secret = !auth_api_secrets.is_empty();

    if !has_jwt_auth && !has_api_secret {
        return Err(
            "When AUTH_REQUIRED=true, either (AUTH_SERVICE_URL + AUTH_SIGNING_KEY_PATH) or AUTH_API_SECRETS_JSON/AUTH_API_SECRET must be configured".into()
        );
    }

    Ok(())
}

/// Validate API secret authentication entries
///
/// Ensures that:
/// - ids are non-empty, trimmed, and unique (case-insensitive)
/// - secrets are non-empty and not reused across ids
pub fn validate_auth_api_secrets(
    auth_api_secrets: &[AuthApiSecret],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut seen_ids = std::collections::HashSet::new();
    let mut seen_secrets = std::collections::HashMap::new();

    for (index, entry) in auth_api_secrets.iter().enumerate() {
        let id_trimmed = entry.id.trim();
        if id_trimmed.is_empty() {
            return Err(format!(
                "Auth API secret id at index {} cannot be empty or whitespace-only",
                index
            )
            .into());
        }
        if id_trimmed != entry.id {
            return Err(format!(
                "Auth API secret id '{}' must not include leading or trailing whitespace",
                entry.id
            )
            .into());
        }

        let id_key = id_trimmed.to_lowercase();
        if !seen_ids.insert(id_key) {
            return Err(format!(
                "Duplicate auth API secret id '{}'; ids are case-insensitive",
                entry.id
            )
            .into());
        }

        if entry.secret.trim().is_empty() {
            return Err(format!(
                "Auth API secret for id '{}' cannot be empty or whitespace-only",
                entry.id
            )
            .into());
        }

        if let Some(existing_id) = seen_secrets.insert(entry.secret.clone(), entry.id.clone()) {
            return Err(format!(
                "Auth API secret for id '{}' duplicates the secret used by id '{}'",
                entry.id, existing_id
            )
            .into());
        }
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
/// - outbound_address (if provided) is non-empty and not whitespace-only
/// - when auth_required is true, auth_id must be non-empty for all hooks
///
/// # Arguments
/// * `sip` - Optional SIP configuration to validate
/// * `auth_required` - When true, enforces non-empty auth_id for all hooks
pub fn validate_sip_config(
    sip: &Option<SipConfig>,
    auth_required: bool,
) -> Result<(), Box<dyn std::error::Error>> {
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

        // Validate auth_id only when auth_required is true
        if auth_required {
            validate_hook_auth_id(&hook.auth_id, &hook.host)?;
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

    // Validate outbound_address if provided
    if let Some(outbound_addr) = &config.outbound_address {
        validate_outbound_address(outbound_addr)?;
    }

    // Validate outbound auth credentials
    validate_outbound_auth(
        config.outbound_auth_username.as_deref(),
        config.outbound_auth_password.as_deref(),
    )?;

    Ok(())
}

/// Validate outbound SIP address
///
/// Ensures that:
/// - Address is not empty or whitespace-only
fn validate_outbound_address(address: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Check for empty or whitespace-only
    if address.trim().is_empty() {
        return Err("SIP outbound_address cannot be empty or whitespace-only".into());
    }

    Ok(())
}

/// Validate outbound trunk authentication credentials
///
/// Ensures that:
/// - If either username or password is set, both must be set
/// - Both must be non-empty and not whitespace-only
fn validate_outbound_auth(
    username: Option<&str>,
    password: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    match (username, password) {
        (Some(user), Some(pass)) => {
            // Both are set - validate them
            if user.trim().is_empty() {
                return Err("SIP outbound_auth_username cannot be empty or whitespace-only".into());
            }
            if pass.trim().is_empty() {
                return Err("SIP outbound_auth_password cannot be empty or whitespace-only".into());
            }
            Ok(())
        }
        (Some(_), None) => {
            Err("SIP outbound_auth_password is required when outbound_auth_username is set".into())
        }
        (None, Some(_)) => {
            Err("SIP outbound_auth_username is required when outbound_auth_password is set".into())
        }
        (None, None) => {
            // Both are None - that's valid (auth is optional)
            Ok(())
        }
    }
}

/// Validate a hook auth_id
///
/// Ensures that:
/// - auth_id is not empty or whitespace-only
fn validate_hook_auth_id(
    auth_id: &str,
    hook_identifier: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Check for empty or whitespace-only
    if auth_id.trim().is_empty() {
        return Err(format!(
            "SIP hook auth_id for '{}' cannot be empty or whitespace-only",
            hook_identifier
        )
        .into());
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
        // Should pass regardless of auth_required
        let result = validate_sip_config(&None, true);
        assert!(result.is_ok());
        let result = validate_sip_config(&None, false);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_auth_api_secrets_valid() {
        let secrets = vec![
            AuthApiSecret {
                id: "client-a".to_string(),
                secret: "token-a".to_string(),
            },
            AuthApiSecret {
                id: "client-b".to_string(),
                secret: "token-b".to_string(),
            },
        ];

        assert!(validate_auth_api_secrets(&secrets).is_ok());
    }

    #[test]
    fn test_validate_auth_api_secrets_duplicate_id_case_insensitive() {
        let secrets = vec![
            AuthApiSecret {
                id: "Client-A".to_string(),
                secret: "token-a".to_string(),
            },
            AuthApiSecret {
                id: "client-a".to_string(),
                secret: "token-b".to_string(),
            },
        ];

        let result = validate_auth_api_secrets(&secrets);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Duplicate auth API secret id")
        );
    }

    #[test]
    fn test_validate_auth_api_secrets_duplicate_secret() {
        let secrets = vec![
            AuthApiSecret {
                id: "client-a".to_string(),
                secret: "shared-token".to_string(),
            },
            AuthApiSecret {
                id: "client-b".to_string(),
                secret: "shared-token".to_string(),
            },
        ];

        let result = validate_auth_api_secrets(&secrets);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("duplicates the secret used by id")
        );
    }

    #[test]
    fn test_validate_auth_api_secrets_empty_id_or_secret() {
        let secrets = vec![
            AuthApiSecret {
                id: "   ".to_string(),
                secret: "token-a".to_string(),
            },
            AuthApiSecret {
                id: "client-b".to_string(),
                secret: "   ".to_string(),
            },
        ];

        let result = validate_auth_api_secrets(&secrets);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("cannot be empty or whitespace-only")
        );
    }

    #[test]
    fn test_validate_auth_required_without_auth_methods() {
        let auth_service_url: Option<String> = None;
        let auth_signing_key_path: Option<PathBuf> = None;
        let auth_api_secrets: Vec<AuthApiSecret> = Vec::new();

        let result = validate_auth_required(
            true,
            &auth_service_url,
            &auth_signing_key_path,
            &auth_api_secrets,
        );

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("AUTH_REQUIRED=true")
        );
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
                auth_id: "tenant-123".to_string(),
            }],
            Some("global-secret-1234567890".to_string()),
            None,
            None,
            None,
        );

        // Should pass with auth_required=true (auth_id is set)
        let result = validate_sip_config(&Some(config.clone()), true);
        assert!(result.is_ok());
        // Should also pass with auth_required=false
        let result = validate_sip_config(&Some(config), false);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_sip_config_empty_room_prefix() {
        let config = SipConfig::new(
            "".to_string(),
            vec!["192.168.1.0/24".to_string()],
            vec![],
            None,
            None,
            None,
            None,
        );

        let result = validate_sip_config(&Some(config), false);
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
            None,
            None,
            None,
        );

        let result = validate_sip_config(&Some(config), false);
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
        let config = SipConfig::new("sip-".to_string(), vec![], vec![], None, None, None, None);

        let result = validate_sip_config(&Some(config), false);
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
            None,
            None,
            None,
        );

        let result = validate_sip_config(&Some(config), false);
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
                    auth_id: "tenant-1".to_string(),
                },
                SipHookConfig {
                    host: "Example.COM".to_string(), // case-insensitive duplicate
                    url: "https://webhook2.example.com/events".to_string(),
                    secret: None,
                    auth_id: "tenant-2".to_string(),
                },
            ],
            Some("global-secret-1234567890".to_string()),
            None,
            None,
            None,
        );

        let result = validate_sip_config(&Some(config), true);
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
            None,
            None,
            None,
        );

        let result = validate_sip_config(&Some(config), false);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_sip_config_valid_alphanumeric_prefix() {
        let config = SipConfig::new(
            "sip-call_123".to_string(),
            vec!["192.168.1.0/24".to_string()],
            vec![],
            None,
            None,
            None,
            None,
        );

        let result = validate_sip_config(&Some(config), false);
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
                auth_id: "tenant-123".to_string(),
            }],
            None, // No global secret
            None,
            None,
            None,
        );

        let result = validate_sip_config(&Some(config), true);
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
                auth_id: "tenant-123".to_string(),
            }],
            Some("short".to_string()), // Too short
            None,
            None,
            None,
        );

        let result = validate_sip_config(&Some(config), true);
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
                auth_id: "tenant-123".to_string(),
            }],
            Some("                ".to_string()), // Whitespace only
            None,
            None,
            None,
        );

        let result = validate_sip_config(&Some(config), true);
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
                    auth_id: "tenant-1".to_string(),
                },
                SipHookConfig {
                    host: "override.com".to_string(),
                    url: "https://webhook.override.com/events".to_string(),
                    secret: Some("per-hook-secret-1234567890".to_string()), // Override
                    auth_id: "tenant-2".to_string(),
                },
            ],
            Some("global-secret-1234567890".to_string()),
            None,
            None,
            None,
        );

        let result = validate_sip_config(&Some(config), true);
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
                auth_id: "tenant-123".to_string(),
            }],
            Some("global-secret-1234567890".to_string()),
            None,
            None,
            None,
        );

        let result = validate_sip_config(&Some(config), true);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must be at least 16 characters")
        );
    }

    // auth_id validation tests

    #[test]
    fn test_validate_sip_config_auth_id_empty_when_auth_required() {
        let config = SipConfig::new(
            "sip-".to_string(),
            vec!["192.168.1.0/24".to_string()],
            vec![SipHookConfig {
                host: "example.com".to_string(),
                url: "https://webhook.example.com/events".to_string(),
                secret: None,
                auth_id: "".to_string(), // Empty auth_id
            }],
            Some("global-secret-1234567890".to_string()),
            None,
            None,
            None,
        );

        // Should fail when auth_required=true
        let result = validate_sip_config(&Some(config.clone()), true);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("auth_id for 'example.com' cannot be empty or whitespace-only")
        );

        // Should pass when auth_required=false
        let result = validate_sip_config(&Some(config), false);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_sip_config_auth_id_whitespace_only_when_auth_required() {
        let config = SipConfig::new(
            "sip-".to_string(),
            vec!["192.168.1.0/24".to_string()],
            vec![SipHookConfig {
                host: "example.com".to_string(),
                url: "https://webhook.example.com/events".to_string(),
                secret: None,
                auth_id: "   ".to_string(), // Whitespace only auth_id
            }],
            Some("global-secret-1234567890".to_string()),
            None,
            None,
            None,
        );

        // Should fail when auth_required=true
        let result = validate_sip_config(&Some(config.clone()), true);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("auth_id for 'example.com' cannot be empty or whitespace-only")
        );

        // Should pass when auth_required=false
        let result = validate_sip_config(&Some(config), false);
        assert!(result.is_ok());
    }

    // Outbound address validation tests

    #[test]
    fn test_validate_sip_config_valid_outbound_address() {
        let config = SipConfig::new(
            "sip-".to_string(),
            vec!["192.168.1.0/24".to_string()],
            vec![],
            None,
            Some("sip.example.com:5060".to_string()),
            None,
            None,
        );

        let result = validate_sip_config(&Some(config), false);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_sip_config_outbound_address_empty() {
        let config = SipConfig::new(
            "sip-".to_string(),
            vec!["192.168.1.0/24".to_string()],
            vec![],
            None,
            Some("".to_string()),
            None,
            None,
        );

        let result = validate_sip_config(&Some(config), false);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("outbound_address cannot be empty or whitespace-only")
        );
    }

    #[test]
    fn test_validate_sip_config_outbound_address_whitespace_only() {
        let config = SipConfig::new(
            "sip-".to_string(),
            vec!["192.168.1.0/24".to_string()],
            vec![],
            None,
            Some("   ".to_string()),
            None,
            None,
        );

        let result = validate_sip_config(&Some(config), false);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("outbound_address cannot be empty or whitespace-only")
        );
    }

    #[test]
    fn test_validate_sip_config_outbound_address_none_is_valid() {
        let config = SipConfig::new(
            "sip-".to_string(),
            vec!["192.168.1.0/24".to_string()],
            vec![],
            None,
            None, // No outbound address is valid (for inbound-only deployments)
            None,
            None,
        );

        let result = validate_sip_config(&Some(config), false);
        assert!(result.is_ok());
    }

    // Outbound auth validation tests

    #[test]
    fn test_validate_sip_config_outbound_auth_both_set() {
        let config = SipConfig::new(
            "sip-".to_string(),
            vec!["192.168.1.0/24".to_string()],
            vec![],
            None,
            Some("sip.example.com".to_string()),
            Some("my-user".to_string()),
            Some("my-pass".to_string()),
        );

        let result = validate_sip_config(&Some(config), false);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_sip_config_outbound_auth_username_only() {
        let config = SipConfig::new(
            "sip-".to_string(),
            vec!["192.168.1.0/24".to_string()],
            vec![],
            None,
            Some("sip.example.com".to_string()),
            Some("my-user".to_string()),
            None, // Password missing
        );

        let result = validate_sip_config(&Some(config), false);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("outbound_auth_password is required when outbound_auth_username is set")
        );
    }

    #[test]
    fn test_validate_sip_config_outbound_auth_password_only() {
        let config = SipConfig::new(
            "sip-".to_string(),
            vec!["192.168.1.0/24".to_string()],
            vec![],
            None,
            Some("sip.example.com".to_string()),
            None, // Username missing
            Some("my-pass".to_string()),
        );

        let result = validate_sip_config(&Some(config), false);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("outbound_auth_username is required when outbound_auth_password is set")
        );
    }

    #[test]
    fn test_validate_sip_config_outbound_auth_username_whitespace_only() {
        let config = SipConfig::new(
            "sip-".to_string(),
            vec!["192.168.1.0/24".to_string()],
            vec![],
            None,
            Some("sip.example.com".to_string()),
            Some("   ".to_string()), // Whitespace only
            Some("my-pass".to_string()),
        );

        let result = validate_sip_config(&Some(config), false);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("outbound_auth_username cannot be empty or whitespace-only")
        );
    }

    #[test]
    fn test_validate_sip_config_outbound_auth_password_whitespace_only() {
        let config = SipConfig::new(
            "sip-".to_string(),
            vec!["192.168.1.0/24".to_string()],
            vec![],
            None,
            Some("sip.example.com".to_string()),
            Some("my-user".to_string()),
            Some("   ".to_string()), // Whitespace only
        );

        let result = validate_sip_config(&Some(config), false);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("outbound_auth_password cannot be empty or whitespace-only")
        );
    }

    #[test]
    fn test_validate_sip_config_outbound_auth_none_is_valid() {
        let config = SipConfig::new(
            "sip-".to_string(),
            vec!["192.168.1.0/24".to_string()],
            vec![],
            None,
            Some("sip.example.com".to_string()),
            None, // No auth is valid
            None,
        );

        let result = validate_sip_config(&Some(config), false);
        assert!(result.is_ok());
    }
}
