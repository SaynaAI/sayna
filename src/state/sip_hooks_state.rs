use std::path::Path;

use tracing::{debug, warn};

use crate::config::{SipConfig, SipHookConfig};
use crate::utils::sip_hooks::{self, CachedSipHook, merge_hooks_with_secrets};

/// Runtime state for SIP hooks with preserved secrets.
#[derive(Debug, Clone)]
pub struct SipHooksState {
    hooks: Vec<SipHookConfig>,
    global_secret: Option<String>,
    room_prefix: String,
}

impl SipHooksState {
    /// Build runtime SIP hooks state by merging config hooks with the cache file.
    pub async fn new(sip_config: &SipConfig, cache_path: Option<&Path>) -> Self {
        let cached_hooks = if let Some(path) = cache_path {
            match sip_hooks::read_hooks_cache(path).await {
                Ok(hooks) => hooks,
                Err(e) => {
                    warn!(
                        cache_dir = %path.display(),
                        error = %e,
                        "Failed to read SIP hooks cache, falling back to config hooks"
                    );
                    vec![]
                }
            }
        } else {
            vec![]
        };

        let hooks = merge_hooks_with_secrets(&sip_config.hooks, &cached_hooks);

        debug!(
            cache_dir = cache_path.map(|p| p.display().to_string()),
            hooks_from_cache = cached_hooks.len(),
            hooks_total = hooks.len(),
            "Initialized SIP hooks state"
        );

        Self {
            hooks,
            global_secret: sip_config.hook_secret.clone(),
            room_prefix: sip_config.room_prefix.clone(),
        }
    }

    /// Get all configured SIP hooks.
    pub fn get_hooks(&self) -> &[SipHookConfig] {
        &self.hooks
    }

    /// Find a hook matching the given domain (case-insensitive).
    pub fn get_hook_for_domain(&self, domain: &str) -> Option<&SipHookConfig> {
        self.hooks
            .iter()
            .find(|hook| hook.host.eq_ignore_ascii_case(domain))
    }

    /// Resolve the signing secret for the given hook, falling back to the global secret.
    pub fn get_signing_secret<'a>(&'a self, hook: &'a SipHookConfig) -> Option<&'a str> {
        hook.secret.as_deref().or(self.global_secret.as_deref())
    }

    /// Get the SIP room prefix.
    pub fn get_room_prefix(&self) -> &str {
        &self.room_prefix
    }

    /// Update the runtime hooks, preserving secrets from the original config.
    pub fn update_hooks(
        &mut self,
        new_hooks: Vec<CachedSipHook>,
        original_config_hooks: &[SipHookConfig],
    ) -> Vec<SipHookConfig> {
        let merged = merge_hooks_with_secrets(original_config_hooks, &new_hooks);
        self.hooks = merged.clone();
        merged
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_sip_hooks_state_initialization_empty_cache() {
        let config_hooks = vec![SipHookConfig {
            host: "example.com".to_string(),
            url: "https://existing.com".to_string(),
            secret: Some("secret-1".to_string()),
        }];
        let sip_config = SipConfig {
            room_prefix: "sip-".to_string(),
            allowed_addresses: vec![],
            hooks: config_hooks.clone(),
            hook_secret: Some("global".to_string()),
        };

        let temp_dir = TempDir::new().unwrap();
        let state = SipHooksState::new(&sip_config, Some(temp_dir.path())).await;

        assert_eq!(state.get_hooks().len(), 1);
        assert_eq!(state.get_hooks()[0].url, "https://existing.com");
        assert_eq!(
            state.get_signing_secret(&state.get_hooks()[0]),
            Some("secret-1")
        );
    }

    #[tokio::test]
    async fn test_sip_hooks_state_initialization_with_cache() {
        let config_hooks = vec![SipHookConfig {
            host: "example.com".to_string(),
            url: "https://existing.com".to_string(),
            secret: Some("secret-1".to_string()),
        }];
        let sip_config = SipConfig {
            room_prefix: "sip-".to_string(),
            allowed_addresses: vec![],
            hooks: config_hooks,
            hook_secret: Some("global".to_string()),
        };

        let temp_dir = TempDir::new().unwrap();
        let cached = vec![
            CachedSipHook::new("example.com", "https://cached.com"),
            CachedSipHook::new("new.com", "https://new.com/hook"),
        ];
        sip_hooks::write_hooks_cache(temp_dir.path(), &cached)
            .await
            .unwrap();

        let state = SipHooksState::new(&sip_config, Some(temp_dir.path())).await;

        assert_eq!(state.get_hooks().len(), 2);
        let hook = state.get_hook_for_domain("example.com").unwrap();
        assert_eq!(hook.url, "https://existing.com");
        assert_eq!(hook.secret.as_deref(), Some("secret-1"));

        let new_hook = state.get_hook_for_domain("new.com").unwrap();
        assert_eq!(new_hook.url, "https://new.com/hook");
        assert!(new_hook.secret.is_none());
    }

    #[tokio::test]
    async fn test_sip_hooks_state_new_hooks_use_global_secret() {
        let sip_config = SipConfig {
            room_prefix: "sip-".to_string(),
            allowed_addresses: vec![],
            hooks: vec![],
            hook_secret: Some("global-secret".to_string()),
        };

        let temp_dir = TempDir::new().unwrap();
        let cached = vec![CachedSipHook::new("cached.com", "https://cached.com/hook")];
        sip_hooks::write_hooks_cache(temp_dir.path(), &cached)
            .await
            .unwrap();

        let state = SipHooksState::new(&sip_config, Some(temp_dir.path())).await;
        let hook = state.get_hook_for_domain("cached.com").unwrap();

        assert_eq!(state.get_signing_secret(hook), Some("global-secret"));
    }

    #[tokio::test]
    async fn test_sip_hooks_state_update_at_runtime() {
        let config_hooks = vec![SipHookConfig {
            host: "example.com".to_string(),
            url: "https://existing.com".to_string(),
            secret: Some("secret-1".to_string()),
        }];
        let sip_config = SipConfig {
            room_prefix: "sip-".to_string(),
            allowed_addresses: vec![],
            hooks: config_hooks.clone(),
            hook_secret: Some("global".to_string()),
        };

        let state = SipHooksState::new(&sip_config, None).await;
        let mut state = state;
        let updated = vec![
            CachedSipHook::new("example.com", "https://updated.com"),
            CachedSipHook::new("runtime.com", "https://runtime.com"),
        ];

        let result = state.update_hooks(updated, &config_hooks);
        assert_eq!(result.len(), 2);

        let config_hook = state.get_hook_for_domain("example.com").unwrap();
        assert_eq!(config_hook.url, "https://existing.com");
        assert_eq!(state.get_signing_secret(config_hook), Some("secret-1"));

        let runtime_hook = state.get_hook_for_domain("runtime.com").unwrap();
        assert_eq!(runtime_hook.url, "https://runtime.com");
        assert_eq!(state.get_signing_secret(runtime_hook), Some("global"));
    }

    #[tokio::test]
    async fn test_sip_hooks_state_case_insensitive_lookup() {
        let config_hooks = vec![SipHookConfig {
            host: "Example.COM".to_string(),
            url: "https://existing.com".to_string(),
            secret: None,
        }];
        let sip_config = SipConfig {
            room_prefix: "sip-".to_string(),
            allowed_addresses: vec![],
            hooks: config_hooks,
            hook_secret: Some("global".to_string()),
        };

        let state = SipHooksState::new(&sip_config, None).await;
        let hook = state.get_hook_for_domain("example.com").unwrap();
        assert_eq!(hook.url, "https://existing.com");
        assert_eq!(hook.host, "example.com");
    }
}
