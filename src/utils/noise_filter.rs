//! Noise reduction utilities using DeepFilterNet.
//!
//! This module re-exports the async interfaces from `core::noise_filter::pool`:
//!
//! - **`StreamNoiseProcessor`**: Per-stream processor for real-time audio streaming
//!   (e.g., LiveKit). Each stream gets a dedicated `NoiseFilter` instance.
//!
//! - **`reduce_noise_async`**: Worker pool for batch/one-shot processing.
//!
//! For implementation details, see `crate::core::noise_filter::pool`.

// Re-export from core module
#[cfg(feature = "noise-filter")]
pub use crate::core::noise_filter::pool::{reduce_noise_async, StreamNoiseProcessor};

// Stub implementations when noise-filter feature is disabled
#[cfg(not(feature = "noise-filter"))]
mod stub {
    use bytes::Bytes;

    #[inline]
    pub async fn reduce_noise_async(
        pcm: Bytes,
        _sample_rate: u32,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(pcm.to_vec())
    }

    /// Stub version of StreamNoiseProcessor that passes audio through unchanged.
    pub struct StreamNoiseProcessor;

    impl StreamNoiseProcessor {
        /// Create a stub processor (no-op).
        pub async fn new(
            _sample_rate: u32,
        ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
            Ok(Self)
        }

        /// Pass audio through unchanged.
        pub async fn process(
            &self,
            pcm: Bytes,
            _sample_rate: u32,
        ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(pcm.to_vec())
        }
    }
}

#[cfg(not(feature = "noise-filter"))]
pub use stub::{reduce_noise_async, StreamNoiseProcessor};
