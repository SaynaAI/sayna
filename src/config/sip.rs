//! SIP configuration structures
//!
//! This module defines SIP-specific configuration including room prefixes,
//! allowed IP addresses for SIP connections, and downstream webhook targets.

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
}

/// SIP configuration
///
/// Contains all SIP-related settings for the Sayna server:
/// - Room name prefix for SIP calls
/// - Allowed IP addresses/CIDRs for SIP connections
/// - Downstream webhook targets for event forwarding
#[derive(Debug, Clone)]
pub struct SipConfig {
    /// Prefix for SIP room names (alphanumeric, '-', '_')
    pub room_prefix: String,
    /// Allowed IP addresses or CIDR ranges for SIP connections
    pub allowed_addresses: Vec<String>,
    /// Downstream webhook configurations (host -> URL)
    pub hooks: Vec<SipHookConfig>,
}

impl SipConfig {
    /// Create a new SIP configuration with normalized values
    ///
    /// This normalizes hostnames to lowercase for consistent matching
    /// and trims whitespace from addresses.
    pub fn new(
        room_prefix: String,
        allowed_addresses: Vec<String>,
        hooks: Vec<SipHookConfig>,
    ) -> Self {
        // Normalize addresses (trim whitespace)
        let allowed_addresses = allowed_addresses
            .into_iter()
            .map(|addr| addr.trim().to_string())
            .collect();

        // Normalize hook hostnames to lowercase
        let hooks = hooks
            .into_iter()
            .map(|hook| SipHookConfig {
                host: hook.host.to_lowercase(),
                url: hook.url,
            })
            .collect();

        Self {
            room_prefix,
            allowed_addresses,
            hooks,
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
        };

        assert_eq!(hook.host, "example.com");
        assert_eq!(hook.url, "https://webhook.example.com/events");
    }

    #[test]
    fn test_sip_config_new() {
        let hooks = vec![SipHookConfig {
            host: "example.com".to_string(),
            url: "https://webhook.example.com/events".to_string(),
        }];

        let config = SipConfig::new(
            "sip-".to_string(),
            vec!["192.168.1.0/24".to_string(), " 10.0.0.1 ".to_string()],
            hooks,
        );

        assert_eq!(config.room_prefix, "sip-");
        assert_eq!(config.allowed_addresses.len(), 2);
        assert_eq!(config.allowed_addresses[0], "192.168.1.0/24");
        assert_eq!(config.allowed_addresses[1], "10.0.0.1"); // trimmed
        assert_eq!(config.hooks.len(), 1);
        assert_eq!(config.hooks[0].host, "example.com");
    }

    #[test]
    fn test_sip_config_normalizes_hostnames() {
        let hooks = vec![
            SipHookConfig {
                host: "Example.COM".to_string(),
                url: "https://webhook1.example.com/events".to_string(),
            },
            SipHookConfig {
                host: "Another.HOST".to_string(),
                url: "https://webhook2.example.com/events".to_string(),
            },
        ];

        let config = SipConfig::new("sip-".to_string(), vec![], hooks);

        assert_eq!(config.hooks[0].host, "example.com"); // normalized to lowercase
        assert_eq!(config.hooks[1].host, "another.host"); // normalized to lowercase
    }

    #[test]
    fn test_sip_config_trims_addresses() {
        let config = SipConfig::new(
            "sip-".to_string(),
            vec!["  192.168.1.0/24  ".to_string(), "\t10.0.0.1\n".to_string()],
            vec![],
        );

        assert_eq!(config.allowed_addresses[0], "192.168.1.0/24");
        assert_eq!(config.allowed_addresses[1], "10.0.0.1");
    }
}
