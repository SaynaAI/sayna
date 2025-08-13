//! Cache module for high-performance data caching.
//!
//! This module provides a unified caching interface with support for
//! multiple backends (memory and filesystem), optimized for concurrent
//! access and zero-copy operations.

pub mod store;

pub use store::{
    CacheBackend, CacheConfig, CacheError, CacheMetrics, CacheStore, FilesystemCacheBackend,
    KeyHasher, MemoryCacheBackend, Result, XxHasher,
};
