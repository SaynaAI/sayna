//! Cache store implementation with multiple backend support.
//!
//! This module provides a unified caching interface with support for
//! memory and filesystem backends, optimized for high-performance
//! concurrent access with zero-copy operations.

use async_trait::async_trait;
use bytes::Bytes;
use moka::future::{Cache as MokaCache, CacheBuilder as MokaCacheBuilder};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::{debug, warn};
use xxhash_rust::xxh3::xxh3_128;

/// Errors that can occur during cache operations.
#[derive(Error, Debug)]
pub enum CacheError {
    /// I/O error occurred during filesystem operations.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization/deserialization error.
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Backend-specific error.
    #[error("Cache backend error: {0}")]
    Backend(String),

    /// Invalid configuration provided.
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
}

/// Result type for cache operations.
pub type Result<T> = std::result::Result<T, CacheError>;

/// Trait defining the interface for cache backends.
#[async_trait]
pub trait CacheBackend: Send + Sync {
    /// Stores a value with an optional TTL.
    async fn set(&self, key: &str, value: Bytes, ttl: Option<Duration>) -> Result<()>;

    /// Retrieves a value by key.
    async fn get(&self, key: &str) -> Result<Option<Bytes>>;

    /// Checks if a key exists in the cache.
    async fn exists(&self, key: &str) -> Result<bool>;

    /// Deletes a value by key.
    async fn delete(&self, key: &str) -> Result<()>;

    /// Clears all entries from the cache.
    async fn clear(&self) -> Result<()>;

    /// Returns the backend type as a string identifier.
    fn backend_type(&self) -> &str;
}

/// Metrics tracking for cache operations.
#[derive(Debug, Clone)]
pub struct CacheMetrics {
    hits: Arc<RwLock<u64>>,
    misses: Arc<RwLock<u64>>,
    sets: Arc<RwLock<u64>>,
    deletes: Arc<RwLock<u64>>,
}

impl Default for CacheMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl CacheMetrics {
    /// Creates a new metrics instance.
    pub fn new() -> Self {
        Self {
            hits: Arc::new(RwLock::new(0)),
            misses: Arc::new(RwLock::new(0)),
            sets: Arc::new(RwLock::new(0)),
            deletes: Arc::new(RwLock::new(0)),
        }
    }

    /// Records a cache hit.
    pub fn record_hit(&self) {
        *self.hits.write() += 1;
    }

    /// Records a cache miss.
    pub fn record_miss(&self) {
        *self.misses.write() += 1;
    }

    /// Records a set operation.
    pub fn record_set(&self) {
        *self.sets.write() += 1;
    }

    /// Records a delete operation.
    pub fn record_delete(&self) {
        *self.deletes.write() += 1;
    }

    /// Returns current statistics as a tuple (hits, misses, sets, deletes).
    pub fn get_stats(&self) -> (u64, u64, u64, u64) {
        (
            *self.hits.read(),
            *self.misses.read(),
            *self.sets.read(),
            *self.deletes.read(),
        )
    }
}

/// Internal structure for memory cache entries with expiration.
struct CacheEntry {
    data: Bytes,
    expires_at: Option<Instant>,
}

/// Memory-based cache backend using Moka.
pub struct MemoryCacheBackend {
    cache: MokaCache<String, Arc<CacheEntry>>,
    default_ttl: Option<Duration>,
}

impl MemoryCacheBackend {
    /// Creates a new memory cache backend.
    ///
    /// # Arguments
    ///
    /// * `max_entries` - Maximum number of entries to store
    /// * `max_size_bytes` - Optional maximum total size in bytes
    /// * `default_ttl` - Optional default TTL for all entries
    pub fn new(
        max_entries: u64,
        max_size_bytes: Option<u64>,
        default_ttl: Option<Duration>,
    ) -> Self {
        let mut builder = MokaCacheBuilder::new(max_entries).support_invalidation_closures();

        if let Some(max_size) = max_size_bytes {
            builder = builder.weigher(|_key, value: &Arc<CacheEntry>| value.data.len() as u32);
            builder = builder.max_capacity(max_size);
        }

        Self {
            cache: builder.build(),
            default_ttl,
        }
    }
}

#[async_trait]
impl CacheBackend for MemoryCacheBackend {
    async fn set(&self, key: &str, value: Bytes, ttl: Option<Duration>) -> Result<()> {
        let expires_at = ttl.or(self.default_ttl).map(|d| Instant::now() + d);

        let entry = Arc::new(CacheEntry {
            data: value,
            expires_at,
        });

        self.cache.insert(key.to_string(), entry).await;
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Option<Bytes>> {
        if let Some(entry) = self.cache.get(key).await {
            if let Some(expires_at) = entry.expires_at
                && Instant::now() > expires_at
            {
                self.cache.invalidate(key).await;
                return Ok(None);
            }
            Ok(Some(entry.data.clone()))
        } else {
            Ok(None)
        }
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        if let Some(entry) = self.cache.get(key).await {
            if let Some(expires_at) = entry.expires_at
                && Instant::now() > expires_at
            {
                self.cache.invalidate(key).await;
                return Ok(false);
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn delete(&self, key: &str) -> Result<()> {
        self.cache.invalidate(key).await;
        Ok(())
    }

    async fn clear(&self) -> Result<()> {
        self.cache.invalidate_all();
        self.cache.run_pending_tasks().await;
        Ok(())
    }

    fn backend_type(&self) -> &str {
        "memory"
    }
}

/// Filesystem-based cache backend.
pub struct FilesystemCacheBackend {
    base_path: PathBuf,
    default_ttl: Option<Duration>,
}

impl FilesystemCacheBackend {
    /// Creates a new filesystem cache backend.
    ///
    /// # Arguments
    ///
    /// * `base_path` - Base directory for cache storage
    /// * `default_ttl` - Optional default TTL for all entries
    pub async fn new(base_path: PathBuf, default_ttl: Option<Duration>) -> Result<Self> {
        fs::create_dir_all(&base_path).await?;
        Ok(Self {
            base_path,
            default_ttl,
        })
    }

    fn get_file_path(&self, key: &str) -> PathBuf {
        let hash = format!("{:032x}", xxh3_128(key.as_bytes()));
        let dir = &hash[0..2];
        self.base_path.join(dir).join(hash)
    }

    fn get_meta_path(&self, key: &str) -> PathBuf {
        let mut path = self.get_file_path(key);
        path.set_extension("meta");
        path
    }
}

/// Metadata for filesystem cache entries.
#[derive(Serialize, Deserialize)]
struct CacheEntryMeta {
    expires_at: Option<u64>,
    created_at: u64,
    size: usize,
}

#[async_trait]
impl CacheBackend for FilesystemCacheBackend {
    async fn set(&self, key: &str, value: Bytes, ttl: Option<Duration>) -> Result<()> {
        let file_path = self.get_file_path(key);
        let meta_path = self.get_meta_path(key);

        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        // Atomic write using temp file
        let temp_path = file_path.with_extension("tmp");
        let mut file = fs::File::create(&temp_path).await?;
        file.write_all(&value).await?;
        file.sync_all().await?;
        drop(file);

        fs::rename(&temp_path, &file_path).await?;

        // Write metadata
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let expires_at = ttl.or(self.default_ttl).map(|d| now + d.as_secs());

        let meta = CacheEntryMeta {
            expires_at,
            created_at: now,
            size: value.len(),
        };

        let meta_json = serde_json::to_vec(&meta)?;
        let temp_meta_path = meta_path.with_extension("tmp");
        let mut meta_file = fs::File::create(&temp_meta_path).await?;
        meta_file.write_all(&meta_json).await?;
        meta_file.sync_all().await?;
        drop(meta_file);

        fs::rename(&temp_meta_path, &meta_path).await?;

        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Option<Bytes>> {
        let file_path = self.get_file_path(key);
        let meta_path = self.get_meta_path(key);

        let meta_data = match fs::read(&meta_path).await {
            Ok(data) => data,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };

        let meta: CacheEntryMeta = serde_json::from_slice(&meta_data)?;

        // Check expiration
        if let Some(expires_at) = meta.expires_at {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();

            if now > expires_at {
                let _ = fs::remove_file(&file_path).await;
                let _ = fs::remove_file(&meta_path).await;
                return Ok(None);
            }
        }

        match fs::read(&file_path).await {
            Ok(data) => Ok(Some(Bytes::from(data))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let _ = fs::remove_file(&meta_path).await;
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        let meta_path = self.get_meta_path(key);

        let meta_data = match fs::read(&meta_path).await {
            Ok(data) => data,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e.into()),
        };

        let meta: CacheEntryMeta = serde_json::from_slice(&meta_data)?;

        if let Some(expires_at) = meta.expires_at {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();

            if now > expires_at {
                return Ok(false);
            }
        }

        Ok(self.get_file_path(key).exists())
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let file_path = self.get_file_path(key);
        let meta_path = self.get_meta_path(key);

        let _ = fs::remove_file(&file_path).await;
        let _ = fs::remove_file(&meta_path).await;

        Ok(())
    }

    async fn clear(&self) -> Result<()> {
        warn!("Clearing filesystem cache at {:?}", self.base_path);
        let _ = fs::remove_dir_all(&self.base_path).await;
        fs::create_dir_all(&self.base_path).await?;
        Ok(())
    }

    fn backend_type(&self) -> &str {
        "filesystem"
    }
}

/// Trait for key hashing strategies.
pub trait KeyHasher: Send + Sync {
    /// Hashes a key to a string.
    fn hash(&self, key: &str) -> String;
}

/// xxHash-based key hasher.
pub struct XxHasher {
    prefix: String,
}

impl XxHasher {
    /// Creates a new hasher with the given prefix.
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
        }
    }
}

impl KeyHasher for XxHasher {
    fn hash(&self, key: &str) -> String {
        let hash = xxh3_128(key.as_bytes());
        format!("{}:{:032x}", self.prefix, hash)
    }
}

/// Cache configuration options.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum CacheConfig {
    /// Memory-based cache configuration.
    Memory {
        /// Maximum number of entries.
        max_entries: u64,
        /// Optional maximum total size in bytes.
        #[serde(default)]
        max_size_bytes: Option<u64>,
        /// Optional TTL in seconds.
        #[serde(default)]
        ttl_seconds: Option<u64>,
    },
    /// Filesystem-based cache configuration.
    Filesystem {
        /// Base path for cache storage.
        path: PathBuf,
        /// Optional TTL in seconds.
        #[serde(default)]
        ttl_seconds: Option<u64>,
    },
}

impl Default for CacheConfig {
    fn default() -> Self {
        CacheConfig::Memory {
            max_entries: 5_000_000,
            max_size_bytes: Some(500 * 1024 * 1024), // 500MB
            ttl_seconds: Some(30 * 24 * 60 * 60),    // 1 month (30 days)
        }
    }
}

/// Unified cache store with pluggable backends.
pub struct CacheStore {
    backend: Arc<dyn CacheBackend>,
    hasher: Arc<dyn KeyHasher>,
    metrics: Arc<CacheMetrics>,
    use_hashing: bool,
}

impl CacheStore {
    /// Creates a new cache store from configuration.
    pub async fn from_config(config: CacheConfig) -> Result<Self> {
        Self::from_config_with_prefix(config, "cache").await
    }

    /// Creates a new cache store with a custom key prefix.
    pub async fn from_config_with_prefix(config: CacheConfig, prefix: &str) -> Result<Self> {
        let backend: Arc<dyn CacheBackend> = match config {
            CacheConfig::Memory {
                max_entries,
                max_size_bytes,
                ttl_seconds,
            } => {
                let ttl = ttl_seconds.map(Duration::from_secs);
                Arc::new(MemoryCacheBackend::new(max_entries, max_size_bytes, ttl))
            }
            CacheConfig::Filesystem { path, ttl_seconds } => {
                let ttl = ttl_seconds.map(Duration::from_secs);
                Arc::new(FilesystemCacheBackend::new(path, ttl).await?)
            }
        };

        Ok(Self {
            backend,
            hasher: Arc::new(XxHasher::new(prefix)),
            metrics: Arc::new(CacheMetrics::new()),
            use_hashing: true,
        })
    }

    /// Configures whether to use key hashing.
    pub fn with_hashing(mut self, use_hashing: bool) -> Self {
        self.use_hashing = use_hashing;
        self
    }

    fn get_cache_key(&self, key: &str) -> String {
        if self.use_hashing {
            self.hasher.hash(key)
        } else {
            key.to_string()
        }
    }

    /// Stores a value in the cache.
    pub async fn put(&self, key: impl AsRef<str>, value: impl Into<Bytes>) -> Result<()> {
        let cache_key = self.get_cache_key(key.as_ref());
        let bytes = value.into();

        debug!(
            "Storing cache entry: {} (size: {} bytes)",
            key.as_ref(),
            bytes.len()
        );

        self.backend.set(&cache_key, bytes, None).await?;
        self.metrics.record_set();
        Ok(())
    }

    /// Stores a value with a specific TTL.
    pub async fn put_with_ttl(
        &self,
        key: impl AsRef<str>,
        value: impl Into<Bytes>,
        ttl: Duration,
    ) -> Result<()> {
        let cache_key = self.get_cache_key(key.as_ref());
        let bytes = value.into();

        debug!(
            "Storing cache entry with TTL: {} (size: {} bytes, ttl: {:?})",
            key.as_ref(),
            bytes.len(),
            ttl
        );

        self.backend.set(&cache_key, bytes, Some(ttl)).await?;
        self.metrics.record_set();
        Ok(())
    }

    /// Retrieves a value from the cache.
    pub async fn get(&self, key: impl AsRef<str>) -> Result<Option<Bytes>> {
        let cache_key = self.get_cache_key(key.as_ref());

        let result = self.backend.get(&cache_key).await?;

        if result.is_some() {
            debug!("Cache hit: {}", key.as_ref());
            self.metrics.record_hit();
        } else {
            debug!("Cache miss: {}", key.as_ref());
            self.metrics.record_miss();
        }

        Ok(result)
    }

    /// Checks if a key exists in the cache.
    pub async fn exists(&self, key: impl AsRef<str>) -> Result<bool> {
        let cache_key = self.get_cache_key(key.as_ref());
        self.backend.exists(&cache_key).await
    }

    /// Deletes a value from the cache.
    pub async fn delete(&self, key: impl AsRef<str>) -> Result<()> {
        let cache_key = self.get_cache_key(key.as_ref());

        debug!("Deleting cache entry: {}", key.as_ref());

        self.backend.delete(&cache_key).await?;
        self.metrics.record_delete();
        Ok(())
    }

    /// Clears all entries from the cache.
    pub async fn clear(&self) -> Result<()> {
        warn!("Clearing all cache entries");
        self.backend.clear().await
    }

    /// Returns the cache metrics.
    pub fn metrics(&self) -> &CacheMetrics {
        &self.metrics
    }

    /// Returns the backend type identifier.
    pub fn backend_type(&self) -> &str {
        self.backend.backend_type()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_memory_cache_basic_operations() {
        let config = CacheConfig::Memory {
            max_entries: 100,
            max_size_bytes: Some(1024 * 1024),
            ttl_seconds: None,
        };

        let cache = CacheStore::from_config(config).await.unwrap();

        // Test set and get
        cache.put("key1", b"value1".to_vec()).await.unwrap();

        let result = cache.get("key1").await.unwrap();
        assert_eq!(result, Some(Bytes::from("value1")));

        // Test exists
        assert!(cache.exists("key1").await.unwrap());
        assert!(!cache.exists("key2").await.unwrap());

        // Test delete
        cache.delete("key1").await.unwrap();
        assert!(!cache.exists("key1").await.unwrap());
    }

    #[tokio::test]
    async fn test_filesystem_cache_basic_operations() {
        let temp_dir = TempDir::new().unwrap();
        let config = CacheConfig::Filesystem {
            path: temp_dir.path().to_path_buf(),
            ttl_seconds: None,
        };

        let cache = CacheStore::from_config(config).await.unwrap();

        // Test set and get
        cache.put("key1", b"value1".to_vec()).await.unwrap();

        let result = cache.get("key1").await.unwrap();
        assert_eq!(result, Some(Bytes::from("value1")));

        // Test exists
        assert!(cache.exists("key1").await.unwrap());

        // Test delete
        cache.delete("key1").await.unwrap();
        assert!(!cache.exists("key1").await.unwrap());
    }

    #[tokio::test]
    async fn test_cache_with_ttl() {
        let config = CacheConfig::Memory {
            max_entries: 100,
            max_size_bytes: None,
            ttl_seconds: None,
        };

        let cache = CacheStore::from_config(config).await.unwrap();

        // Set with short TTL
        cache
            .put_with_ttl("key1", b"value1".to_vec(), Duration::from_millis(100))
            .await
            .unwrap();

        assert!(cache.exists("key1").await.unwrap());

        // Wait for expiration
        tokio::time::sleep(Duration::from_millis(150)).await;

        assert!(cache.get("key1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_metrics() {
        let config = CacheConfig::Memory {
            max_entries: 100,
            max_size_bytes: None,
            ttl_seconds: None,
        };

        let cache = CacheStore::from_config(config).await.unwrap();

        cache.put("key1", b"value1".to_vec()).await.unwrap();
        let _ = cache.get("key1").await; // Hit
        let _ = cache.get("key2").await; // Miss
        cache.delete("key1").await.unwrap();

        let (hits, misses, sets, deletes) = cache.metrics().get_stats();
        assert_eq!(hits, 1);
        assert_eq!(misses, 1);
        assert_eq!(sets, 1);
        assert_eq!(deletes, 1);
    }

    #[tokio::test]
    async fn test_zero_copy_bytes() {
        let config = CacheConfig::Memory {
            max_entries: 100,
            max_size_bytes: None,
            ttl_seconds: None,
        };

        let cache = CacheStore::from_config(config).await.unwrap();

        let data = Bytes::from(vec![0u8; 1024]);
        cache.put("key1", data.clone()).await.unwrap();

        let result = cache.get("key1").await.unwrap().unwrap();
        assert_eq!(result, data);
    }

    #[tokio::test]
    async fn test_key_hashing() {
        let config = CacheConfig::Memory {
            max_entries: 100,
            max_size_bytes: None,
            ttl_seconds: None,
        };

        let cache = CacheStore::from_config(config).await.unwrap();

        let long_key = "this_is_a_very_long_key_that_would_take_up_space";
        cache.put(long_key, b"data".to_vec()).await.unwrap();

        assert!(cache.exists(long_key).await.unwrap());
        assert_eq!(
            cache.get(long_key).await.unwrap(),
            Some(Bytes::from("data"))
        );
    }

    #[tokio::test]
    async fn test_cache_clear() {
        let config = CacheConfig::Memory {
            max_entries: 100,
            max_size_bytes: None,
            ttl_seconds: None,
        };

        let cache = CacheStore::from_config(config).await.unwrap();

        cache.put("key1", b"value1".to_vec()).await.unwrap();
        cache.put("key2", b"value2".to_vec()).await.unwrap();

        cache.clear().await.unwrap();

        assert!(!cache.exists("key1").await.unwrap());
        assert!(!cache.exists("key2").await.unwrap());
    }
}
