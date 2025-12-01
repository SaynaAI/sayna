//! SIP hooks cache file operations
//!
//! This module provides utilities for reading and writing SIP hook configurations
//! to a cache file (`sip_hooks.json`). This enables runtime modification of hooks
//! that persist across server restarts.
//!
//! The cache file stores a simple list of host-to-URL mappings. Note that secrets
//! are NOT stored in the cache file - they must be configured via YAML/ENV.
//! Runtime hooks without secrets will use the global `hook_secret` from config.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use thiserror::Error;
use tokio::fs;
use tracing::{debug, info, warn};

use crate::config::SipHookConfig;
/// Name of the cache file for SIP hooks
pub const SIP_HOOKS_CACHE_FILE: &str = "sip_hooks.json";

/// Errors that can occur during SIP hooks cache operations.
#[derive(Error, Debug)]
pub enum SipHooksCacheError {
    /// I/O error occurred during filesystem operations.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization/deserialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Validation error for hook data.
    #[error("Validation error: {0}")]
    Validation(String),
}

/// Result type for SIP hooks cache operations.
pub type Result<T> = std::result::Result<T, SipHooksCacheError>;

/// A single SIP hook entry for cache storage.
///
/// This is a simplified version of `SipHookConfig` without the secret field,
/// as secrets should not be stored in the cache file for security reasons.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CachedSipHook {
    /// Host pattern for matching (will be normalized to lowercase)
    pub host: String,
    /// HTTPS URL to forward webhook events to
    pub url: String,
}

impl CachedSipHook {
    /// Creates a new cached SIP hook with normalized host.
    pub fn new(host: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            host: host.into().trim().to_lowercase(),
            url: url.into(),
        }
    }

    /// Normalizes the host to lowercase.
    pub fn normalize(&mut self) {
        self.host = self.host.trim().to_lowercase();
    }
}

/// Validates a list of SIP hooks for duplicates.
///
/// Performs case-insensitive comparison of hosts to detect duplicates.
///
/// # Arguments
/// * `hooks` - The list of hooks to validate
///
/// # Returns
/// * `Ok(())` if no duplicates found
/// * `Err(SipHooksCacheError::Validation)` if duplicates detected
pub fn validate_no_duplicate_hosts(hooks: &[CachedSipHook]) -> Result<()> {
    let mut seen_hosts = HashSet::new();

    for hook in hooks {
        let host_lower = hook.host.to_lowercase();
        if !seen_hosts.insert(host_lower.clone()) {
            return Err(SipHooksCacheError::Validation(format!(
                "Duplicate host detected: {}",
                host_lower
            )));
        }
    }

    Ok(())
}

/// Reads SIP hooks from the cache file.
///
/// # Arguments
/// * `cache_dir` - Path to the cache directory
///
/// # Returns
/// * `Ok(Vec<CachedSipHook>)` - List of cached hooks (empty if file doesn't exist)
/// * `Err(SipHooksCacheError)` - If reading or parsing fails
pub async fn read_hooks_cache(cache_dir: &Path) -> Result<Vec<CachedSipHook>> {
    let cache_file = cache_dir.join(SIP_HOOKS_CACHE_FILE);

    if !cache_file.exists() {
        debug!(
            cache_file = %cache_file.display(),
            "SIP hooks cache file does not exist, returning empty list"
        );
        return Ok(vec![]);
    }

    let content = fs::read_to_string(&cache_file).await?;

    if content.trim().is_empty() {
        debug!(
            cache_file = %cache_file.display(),
            "SIP hooks cache file is empty, returning empty list"
        );
        return Ok(vec![]);
    }

    let mut hooks: Vec<CachedSipHook> = serde_json::from_str(&content)?;

    // Normalize all hosts to lowercase
    for hook in &mut hooks {
        hook.normalize();
    }

    debug!(
        cache_file = %cache_file.display(),
        hook_count = hooks.len(),
        "Read SIP hooks from cache"
    );

    Ok(hooks)
}

/// Writes SIP hooks to the cache file.
///
/// The hooks are validated for duplicate hosts before writing.
/// Hosts are normalized to lowercase before saving.
///
/// # Arguments
/// * `cache_dir` - Path to the cache directory
/// * `hooks` - List of hooks to write
///
/// # Returns
/// * `Ok(())` - If writing succeeds
/// * `Err(SipHooksCacheError)` - If validation or writing fails
pub async fn write_hooks_cache(cache_dir: &Path, hooks: &[CachedSipHook]) -> Result<()> {
    // Normalize hosts before validation
    let normalized_hooks: Vec<CachedSipHook> = hooks
        .iter()
        .map(|h| CachedSipHook::new(h.host.clone(), h.url.clone()))
        .collect();

    // Validate no duplicate hosts
    validate_no_duplicate_hosts(&normalized_hooks)?;

    // Ensure cache directory exists
    fs::create_dir_all(cache_dir).await?;

    let cache_file = cache_dir.join(SIP_HOOKS_CACHE_FILE);

    // Serialize with pretty printing for readability
    let content = serde_json::to_string_pretty(&normalized_hooks)?;

    // Atomic write using temp file
    let temp_file = cache_file.with_extension("tmp");
    fs::write(&temp_file, &content).await?;
    fs::rename(&temp_file, &cache_file).await?;

    info!(
        cache_file = %cache_file.display(),
        hook_count = normalized_hooks.len(),
        "Wrote SIP hooks to cache"
    );

    Ok(())
}

/// Merges hooks, giving priority to the `cached_hooks` list when hosts match.
///
/// This helper is used for merging runtime or cached updates with another list
/// of hooks. For merging with application configuration (where config should
/// take precedence), use `merge_hooks_with_secrets`.
///
/// Note: Cached hooks do NOT include secrets. When an override happens, any
/// per-hook secret from the base list is not retained.
///
/// # Arguments
/// * `existing_hooks` - Hooks from the original configuration (with optional secrets)
/// * `cached_hooks` - Hooks from the cache file (no secrets)
///
/// # Returns
/// A merged list where cached hooks take precedence over existing hooks.
/// The original hook's secret is preserved only if the URL matches exactly.
pub fn merge_hooks<T>(existing_hooks: &[T], cached_hooks: &[CachedSipHook]) -> Vec<CachedSipHook>
where
    T: AsRef<str> + SipHookInfo,
{
    use std::collections::HashMap;

    // Build a map of existing hooks (host -> (url, _secret))
    let existing_map: HashMap<String, &T> = existing_hooks
        .iter()
        .map(|h| (h.host().to_lowercase(), h))
        .collect();

    // Build a map of cached hooks (host -> url)
    let cached_map: HashMap<String, &CachedSipHook> = cached_hooks
        .iter()
        .map(|h| (h.host.to_lowercase(), h))
        .collect();

    // Combine: cached hooks override existing
    let mut result_map: HashMap<String, CachedSipHook> = HashMap::new();

    // First, add all existing hooks
    for hook in existing_hooks {
        let host = hook.host().to_lowercase();
        result_map.insert(
            host.clone(),
            CachedSipHook {
                host,
                url: hook.url().to_string(),
            },
        );
    }

    // Then, override with cached hooks
    for hook in cached_hooks {
        let host = hook.host.to_lowercase();
        result_map.insert(host.clone(), hook.clone());
    }

    // Also add any cached hooks that weren't in existing
    for (host, hook) in &cached_map {
        if !existing_map.contains_key(host) {
            result_map.insert(host.clone(), (*hook).clone());
        }
    }

    result_map.into_values().collect()
}

/// Merges cached hooks with existing configuration hooks while preserving secrets.
///
/// Application configuration hooks always take precedence over cached hooks with
/// the same host. Cached hooks only add new entries that do not exist in the base
/// configuration.
pub fn merge_hooks_with_secrets(
    config_hooks: &[SipHookConfig],
    cached_hooks: &[CachedSipHook],
) -> Vec<SipHookConfig> {
    use std::collections::HashMap;

    let mut merged: HashMap<String, SipHookConfig> = HashMap::new();

    for hook in config_hooks {
        let host = hook.host.trim().to_lowercase();
        merged.insert(
            host.clone(),
            SipHookConfig {
                host,
                url: hook.url.clone(),
                secret: hook.secret.clone(),
            },
        );
    }

    for hook in cached_hooks {
        let host = hook.host.trim().to_lowercase();

        // Ignore cached values for hosts defined in application config.
        if merged.contains_key(&host) {
            continue;
        }

        merged.insert(
            host.clone(),
            SipHookConfig {
                host,
                url: hook.url.clone(),
                secret: None,
            },
        );
    }

    merged.into_values().collect()
}

/// Trait for extracting hook information from different types.
///
/// This allows `merge_hooks` to work with both `SipHookConfig` and `CachedSipHook`.
pub trait SipHookInfo {
    fn host(&self) -> &str;
    fn url(&self) -> &str;
}

impl SipHookInfo for CachedSipHook {
    fn host(&self) -> &str {
        &self.host
    }

    fn url(&self) -> &str {
        &self.url
    }
}

impl AsRef<str> for CachedSipHook {
    fn as_ref(&self) -> &str {
        &self.host
    }
}

/// Reads hooks from cache and merges them with existing hooks, giving priority
/// to cached values.
///
/// This is a convenience function that combines `read_hooks_cache` and
/// `merge_hooks`. For application configuration merges, prefer
/// `merge_hooks_with_secrets`.
///
/// # Arguments
/// * `cache_dir` - Path to the cache directory
/// * `existing_hooks` - Existing hooks to merge with
///
/// # Returns
/// * `Ok(Vec<CachedSipHook>)` - Merged hooks (cached overrides existing)
/// * Returns existing hooks unchanged if cache doesn't exist or is empty
pub async fn read_and_merge_hooks<T>(cache_dir: &Path, existing_hooks: &[T]) -> Vec<CachedSipHook>
where
    T: AsRef<str> + SipHookInfo,
{
    let cached_hooks = match read_hooks_cache(cache_dir).await {
        Ok(hooks) => hooks,
        Err(e) => {
            warn!(
                cache_dir = %cache_dir.display(),
                error = %e,
                "Failed to read SIP hooks cache, using existing hooks only"
            );
            // Return existing hooks converted to CachedSipHook
            return existing_hooks
                .iter()
                .map(|h| CachedSipHook::new(h.host(), h.url()))
                .collect();
        }
    };

    if cached_hooks.is_empty() {
        debug!(
            cache_dir = %cache_dir.display(),
            "No cached SIP hooks found, using existing hooks only"
        );
        return existing_hooks
            .iter()
            .map(|h| CachedSipHook::new(h.host(), h.url()))
            .collect();
    }

    let merged = merge_hooks(existing_hooks, &cached_hooks);

    info!(
        cache_dir = %cache_dir.display(),
        existing_hooks = existing_hooks.len(),
        cached_hooks = cached_hooks.len(),
        merged_hooks = merged.len(),
        "Merged SIP hooks from cache"
    );

    merged
}

/// Removes hooks from the cache by their host names (case-insensitive).
///
/// This function filters out hooks whose hosts match any in the provided list.
/// The comparison is case-insensitive.
///
/// # Arguments
/// * `hooks` - The current list of cached hooks
/// * `hosts_to_remove` - List of host names to remove (case-insensitive)
///
/// # Returns
/// A new list with the specified hosts removed.
pub fn remove_hooks_by_hosts(
    hooks: &[CachedSipHook],
    hosts_to_remove: &[String],
) -> Vec<CachedSipHook> {
    let hosts_set: HashSet<String> = hosts_to_remove
        .iter()
        .map(|h| h.trim().to_lowercase())
        .collect();

    hooks
        .iter()
        .filter(|hook| !hosts_set.contains(&hook.host.to_lowercase()))
        .cloned()
        .collect()
}

/// Deletes the SIP hooks cache file if it exists.
///
/// # Arguments
/// * `cache_dir` - Path to the cache directory
///
/// # Returns
/// * `Ok(true)` - If the file was deleted
/// * `Ok(false)` - If the file did not exist
/// * `Err(SipHooksCacheError)` - If deletion fails
pub async fn delete_hooks_cache(cache_dir: &Path) -> Result<bool> {
    let cache_file = cache_dir.join(SIP_HOOKS_CACHE_FILE);

    if !cache_file.exists() {
        return Ok(false);
    }

    fs::remove_file(&cache_file).await?;

    warn!(
        cache_file = %cache_file.display(),
        "Deleted SIP hooks cache file"
    );

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_cached_sip_hook_new_normalizes_host() {
        let hook = CachedSipHook::new("Example.COM", "https://example.com/webhook");
        assert_eq!(hook.host, "example.com");
    }

    #[test]
    fn test_cached_sip_hook_new_trims_host() {
        let hook = CachedSipHook::new("  example.com  ", "https://example.com/webhook");
        assert_eq!(hook.host, "example.com");
    }

    #[test]
    fn test_validate_no_duplicate_hosts_empty() {
        let result = validate_no_duplicate_hosts(&[]);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_no_duplicate_hosts_unique() {
        let hooks = vec![
            CachedSipHook::new("example.com", "https://a.com"),
            CachedSipHook::new("other.com", "https://b.com"),
        ];
        let result = validate_no_duplicate_hosts(&hooks);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_no_duplicate_hosts_duplicate() {
        let hooks = vec![
            CachedSipHook::new("example.com", "https://a.com"),
            CachedSipHook::new("EXAMPLE.COM", "https://b.com"),
        ];
        let result = validate_no_duplicate_hosts(&hooks);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Duplicate host"));
    }

    #[tokio::test]
    async fn test_read_hooks_cache_missing_file() {
        let temp_dir = TempDir::new().unwrap();
        let result = read_hooks_cache(temp_dir.path()).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_read_hooks_cache_empty_file() {
        let temp_dir = TempDir::new().unwrap();
        let cache_file = temp_dir.path().join(SIP_HOOKS_CACHE_FILE);
        fs::write(&cache_file, "").await.unwrap();

        let result = read_hooks_cache(temp_dir.path()).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_write_and_read_hooks_cache() {
        let temp_dir = TempDir::new().unwrap();
        let hooks = vec![
            CachedSipHook::new("example.com", "https://webhook.example.com/events"),
            CachedSipHook::new("another.com", "https://webhook.another.com/events"),
        ];

        // Write
        write_hooks_cache(temp_dir.path(), &hooks).await.unwrap();

        // Read back
        let read_hooks = read_hooks_cache(temp_dir.path()).await.unwrap();
        assert_eq!(read_hooks.len(), 2);

        // Verify content
        let hosts: Vec<_> = read_hooks.iter().map(|h| h.host.as_str()).collect();
        assert!(hosts.contains(&"example.com"));
        assert!(hosts.contains(&"another.com"));
    }

    #[tokio::test]
    async fn test_write_hooks_cache_normalizes_hosts() {
        let temp_dir = TempDir::new().unwrap();
        let hooks = vec![CachedSipHook {
            host: "EXAMPLE.COM".to_string(),
            url: "https://webhook.example.com/events".to_string(),
        }];

        write_hooks_cache(temp_dir.path(), &hooks).await.unwrap();

        let read_hooks = read_hooks_cache(temp_dir.path()).await.unwrap();
        assert_eq!(read_hooks[0].host, "example.com");
    }

    #[tokio::test]
    async fn test_write_hooks_cache_rejects_duplicates() {
        let temp_dir = TempDir::new().unwrap();
        let hooks = vec![
            CachedSipHook::new("example.com", "https://a.com"),
            CachedSipHook::new("EXAMPLE.COM", "https://b.com"),
        ];

        let result = write_hooks_cache(temp_dir.path(), &hooks).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_hooks_cache_nonexistent() {
        let temp_dir = TempDir::new().unwrap();
        let result = delete_hooks_cache(temp_dir.path()).await;
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn test_delete_hooks_cache_exists() {
        let temp_dir = TempDir::new().unwrap();
        let hooks = vec![CachedSipHook::new("example.com", "https://example.com")];
        write_hooks_cache(temp_dir.path(), &hooks).await.unwrap();

        let result = delete_hooks_cache(temp_dir.path()).await;
        assert!(result.is_ok());
        assert!(result.unwrap());

        // Verify file is gone
        let cache_file = temp_dir.path().join(SIP_HOOKS_CACHE_FILE);
        assert!(!cache_file.exists());
    }

    #[test]
    fn test_merge_hooks_empty() {
        let existing: Vec<CachedSipHook> = vec![];
        let cached: Vec<CachedSipHook> = vec![];
        let result = merge_hooks(&existing, &cached);
        assert!(result.is_empty());
    }

    #[test]
    fn test_merge_hooks_only_existing() {
        let existing = vec![CachedSipHook::new("example.com", "https://existing.com")];
        let cached: Vec<CachedSipHook> = vec![];
        let result = merge_hooks(&existing, &cached);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].url, "https://existing.com");
    }

    #[test]
    fn test_merge_hooks_only_cached() {
        let existing: Vec<CachedSipHook> = vec![];
        let cached = vec![CachedSipHook::new("example.com", "https://cached.com")];
        let result = merge_hooks(&existing, &cached);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].url, "https://cached.com");
    }

    #[test]
    fn test_merge_hooks_cached_overrides_existing() {
        let existing = vec![CachedSipHook::new("example.com", "https://existing.com")];
        let cached = vec![CachedSipHook::new("example.com", "https://cached.com")];
        let result = merge_hooks(&existing, &cached);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].url, "https://cached.com");
    }

    #[test]
    fn test_merge_hooks_case_insensitive_override() {
        let existing = vec![CachedSipHook::new("Example.COM", "https://existing.com")];
        let cached = vec![CachedSipHook::new("EXAMPLE.com", "https://cached.com")];
        let result = merge_hooks(&existing, &cached);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].url, "https://cached.com");
    }

    #[test]
    fn test_merge_hooks_combines_different_hosts() {
        let existing = vec![CachedSipHook::new("existing.com", "https://existing.com")];
        let cached = vec![CachedSipHook::new("cached.com", "https://cached.com")];
        let result = merge_hooks(&existing, &cached);
        assert_eq!(result.len(), 2);

        let hosts: Vec<_> = result.iter().map(|h| h.host.as_str()).collect();
        assert!(hosts.contains(&"existing.com"));
        assert!(hosts.contains(&"cached.com"));
    }

    #[test]
    fn test_merge_hooks_with_secrets_preserves_existing() {
        let config_hooks = vec![SipHookConfig {
            host: "example.com".to_string(),
            url: "https://existing.com".to_string(),
            secret: Some("secret-1".to_string()),
        }];

        let result = merge_hooks_with_secrets(&config_hooks, &[]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].url, "https://existing.com");
        assert_eq!(result[0].secret.as_deref(), Some("secret-1"));
    }

    #[test]
    fn test_merge_hooks_with_secrets_does_not_override_config_hooks() {
        let config_hooks = vec![SipHookConfig {
            host: "example.com".to_string(),
            url: "https://existing.com".to_string(),
            secret: Some("secret-1".to_string()),
        }];
        let cached_hooks = vec![CachedSipHook::new("example.com", "https://cached.com")];

        let result = merge_hooks_with_secrets(&config_hooks, &cached_hooks);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].url, "https://existing.com");
        assert_eq!(result[0].secret.as_deref(), Some("secret-1"));
    }

    #[test]
    fn test_merge_hooks_with_secrets_adds_new_hook_without_secret() {
        let config_hooks = vec![SipHookConfig {
            host: "example.com".to_string(),
            url: "https://existing.com".to_string(),
            secret: Some("secret-1".to_string()),
        }];
        let cached_hooks = vec![CachedSipHook::new("new.com", "https://cached.com")];

        let result = merge_hooks_with_secrets(&config_hooks, &cached_hooks);
        assert_eq!(result.len(), 2);
        let new_hook = result
            .iter()
            .find(|h| h.host == "new.com")
            .expect("new hook should be present");
        assert_eq!(new_hook.secret, None);
    }

    #[tokio::test]
    async fn test_read_and_merge_hooks_no_cache() {
        let temp_dir = TempDir::new().unwrap();
        let existing = vec![CachedSipHook::new("example.com", "https://existing.com")];

        let result = read_and_merge_hooks(temp_dir.path(), &existing).await;
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].url, "https://existing.com");
    }

    #[tokio::test]
    async fn test_read_and_merge_hooks_with_cache() {
        let temp_dir = TempDir::new().unwrap();

        // Write cached hooks
        let cached = vec![CachedSipHook::new("new.com", "https://cached.com")];
        write_hooks_cache(temp_dir.path(), &cached).await.unwrap();

        let existing = vec![CachedSipHook::new("example.com", "https://existing.com")];
        let result = read_and_merge_hooks(temp_dir.path(), &existing).await;

        assert_eq!(result.len(), 2);
        let hosts: Vec<_> = result.iter().map(|h| h.host.as_str()).collect();
        assert!(hosts.contains(&"example.com"));
        assert!(hosts.contains(&"new.com"));
    }

    #[tokio::test]
    async fn test_read_and_merge_hooks_override() {
        let temp_dir = TempDir::new().unwrap();

        // Write cached hooks that override existing
        let cached = vec![CachedSipHook::new("example.com", "https://cached.com")];
        write_hooks_cache(temp_dir.path(), &cached).await.unwrap();

        let existing = vec![CachedSipHook::new("example.com", "https://existing.com")];
        let result = read_and_merge_hooks(temp_dir.path(), &existing).await;

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].url, "https://cached.com");
    }

    #[test]
    fn test_remove_hooks_by_hosts_empty_list() {
        let hooks = vec![CachedSipHook::new("example.com", "https://example.com")];
        let result = remove_hooks_by_hosts(&hooks, &[]);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_remove_hooks_by_hosts_removes_matching() {
        let hooks = vec![
            CachedSipHook::new("example.com", "https://example.com"),
            CachedSipHook::new("other.com", "https://other.com"),
        ];
        let result = remove_hooks_by_hosts(&hooks, &["example.com".to_string()]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].host, "other.com");
    }

    #[test]
    fn test_remove_hooks_by_hosts_case_insensitive() {
        let hooks = vec![
            CachedSipHook::new("example.com", "https://example.com"),
            CachedSipHook::new("other.com", "https://other.com"),
        ];
        let result = remove_hooks_by_hosts(&hooks, &["EXAMPLE.COM".to_string()]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].host, "other.com");
    }

    #[test]
    fn test_remove_hooks_by_hosts_removes_multiple() {
        let hooks = vec![
            CachedSipHook::new("a.com", "https://a.com"),
            CachedSipHook::new("b.com", "https://b.com"),
            CachedSipHook::new("c.com", "https://c.com"),
        ];
        let result = remove_hooks_by_hosts(&hooks, &["a.com".to_string(), "c.com".to_string()]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].host, "b.com");
    }

    #[test]
    fn test_remove_hooks_by_hosts_nonexistent_host() {
        let hooks = vec![CachedSipHook::new("example.com", "https://example.com")];
        let result = remove_hooks_by_hosts(&hooks, &["nonexistent.com".to_string()]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].host, "example.com");
    }

    #[test]
    fn test_remove_hooks_by_hosts_trims_whitespace() {
        let hooks = vec![CachedSipHook::new("example.com", "https://example.com")];
        let result = remove_hooks_by_hosts(&hooks, &["  example.com  ".to_string()]);
        assert_eq!(result.len(), 0);
    }
}
