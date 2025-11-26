//! SIP configuration structures
//!
//! This module defines SIP-specific configuration including room prefixes,
//! allowed IP addresses for SIP connections, and downstream webhook targets.

use crate::utils::sip_hooks::SipHookInfo;

/// SIP webhook configuration
///
/// Defines a downstream webhook target for forwarding LiveKit SIP events.
/// Each webhook is associated with a specific host pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SipHookConfig {
    /// Host pattern for matching (normalized to lowercase)
    pub host: String,
    /// HTTPS URL to forward webhook events to
    pub url: String,
    /// Optional per-hook signing secret (overrides global hook_secret)
    pub secret: Option<String>,
}

impl SipHookInfo for SipHookConfig {
    fn host(&self) -> &str {
        &self.host
    }

    fn url(&self) -> &str {
        &self.url
    }
}

impl AsRef<str> for SipHookConfig {
    fn as_ref(&self) -> &str {
        &self.host
    }
}

/// SIP configuration
///
/// Contains all SIP-related settings for the Sayna server:
/// - Room name prefix for SIP calls
/// - Allowed IP addresses/CIDRs for SIP connections
/// - Downstream webhook targets for event forwarding
/// - Global signing secret for webhook requests
#[derive(Debug, Clone)]
pub struct SipConfig {
    /// Prefix for SIP room names (alphanumeric, '-', '_')
    pub room_prefix: String,
    /// Allowed IP addresses or CIDR ranges for SIP connections
    pub allowed_addresses: Vec<String>,
    /// Downstream webhook configurations (host -> URL)
    pub hooks: Vec<SipHookConfig>,
    /// Global signing secret for webhook requests (can be overridden per hook)
    pub hook_secret: Option<String>,
}

impl SipConfig {
    /// Create a new SIP configuration with normalized values
    ///
    /// This normalizes hostnames to lowercase for consistent matching,
    /// trims whitespace from addresses, and normalizes secrets (trim only,
    /// without logging them).
    pub fn new(
        room_prefix: String,
        allowed_addresses: Vec<String>,
        hooks: Vec<SipHookConfig>,
        hook_secret: Option<String>,
    ) -> Self {
        // Normalize addresses (trim whitespace)
        let allowed_addresses = allowed_addresses
            .into_iter()
            .map(|addr| addr.trim().to_string())
            .collect();

        // Normalize hook hostnames to lowercase and trim secrets
        let hooks = hooks
            .into_iter()
            .map(|hook| SipHookConfig {
                host: hook.host.trim().to_lowercase(),
                url: hook.url,
                secret: hook.secret.map(|s| s.trim().to_string()),
            })
            .collect();

        // Normalize global secret (trim only, no logging)
        let hook_secret = hook_secret.map(|s| s.trim().to_string());

        Self {
            room_prefix,
            allowed_addresses,
            hooks,
            hook_secret,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sip_hook_config() {
        let hook = SipHookConfig {
            host: "example.com".to_string(),
            url: "https://webhook.example.com/events".to_string(),
            secret: Some("test-secret".to_string()),
        };

        assert_eq!(hook.host, "example.com");
        assert_eq!(hook.url, "https://webhook.example.com/events");
        assert_eq!(hook.secret, Some("test-secret".to_string()));
    }

    #[test]
    fn test_sip_config_new() {
        let hooks = vec![SipHookConfig {
            host: "example.com".to_string(),
            url: "https://webhook.example.com/events".to_string(),
            secret: None,
        }];

        let config = SipConfig::new(
            "sip-".to_string(),
            vec!["192.168.1.0/24".to_string(), " 10.0.0.1 ".to_string()],
            hooks,
            Some("global-secret".to_string()),
        );

        assert_eq!(config.room_prefix, "sip-");
        assert_eq!(config.allowed_addresses.len(), 2);
        assert_eq!(config.allowed_addresses[0], "192.168.1.0/24");
        assert_eq!(config.allowed_addresses[1], "10.0.0.1"); // trimmed
        assert_eq!(config.hooks.len(), 1);
        assert_eq!(config.hooks[0].host, "example.com");
        assert_eq!(config.hook_secret, Some("global-secret".to_string()));
    }

    #[test]
    fn test_sip_config_normalizes_hostnames() {
        let hooks = vec![
            SipHookConfig {
                host: "Example.COM".to_string(),
                url: "https://webhook1.example.com/events".to_string(),
                secret: None,
            },
            SipHookConfig {
                host: "Another.HOST".to_string(),
                url: "https://webhook2.example.com/events".to_string(),
                secret: None,
            },
        ];

        let config = SipConfig::new("sip-".to_string(), vec![], hooks, None);

        assert_eq!(config.hooks[0].host, "example.com"); // normalized to lowercase
        assert_eq!(config.hooks[1].host, "another.host"); // normalized to lowercase
    }

    #[test]
    fn test_sip_config_trims_addresses() {
        let config = SipConfig::new(
            "sip-".to_string(),
            vec!["  192.168.1.0/24  ".to_string(), "\t10.0.0.1\n".to_string()],
            vec![],
            None,
        );

        assert_eq!(config.allowed_addresses[0], "192.168.1.0/24");
        assert_eq!(config.allowed_addresses[1], "10.0.0.1");
    }

    #[test]
    fn test_sip_config_normalizes_secrets() {
        let hooks = vec![SipHookConfig {
            host: "example.com".to_string(),
            url: "https://webhook.example.com/events".to_string(),
            secret: Some("  hook-secret  ".to_string()),
        }];

        let config = SipConfig::new(
            "sip-".to_string(),
            vec![],
            hooks,
            Some("  global-secret  ".to_string()),
        );

        assert_eq!(config.hook_secret, Some("global-secret".to_string())); // trimmed
        assert_eq!(config.hooks[0].secret, Some("hook-secret".to_string())); // trimmed
    }
}
