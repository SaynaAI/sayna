//! Async worker pool and stream processor for DeepFilterNet noise filtering.
//!
//! This module provides thread-safe async interfaces for using the `NoiseFilter`:
//!
//! 1. **`StreamNoiseProcessor`**: Per-stream processor for real-time audio streaming
//!    (e.g., LiveKit). Each stream gets a dedicated `NoiseFilter` instance to maintain
//!    proper buffer state across frames.
//!
//! 2. **`reduce_noise_async`**: Worker pool for batch/one-shot processing where
//!    buffer continuity is not required.
//!
//! # Why This Module Exists
//!
//! The `NoiseFilter` is `!Send` because ONNX Runtime sessions cannot be moved between
//! threads. This module spawns dedicated OS threads to pin the sessions, providing
//! async-friendly interfaces for use from Tokio tasks.
//!
//! # Usage
//!
//! For real-time streaming (LiveKit, WebRTC):
//! ```ignore
//! let processor = StreamNoiseProcessor::new(16000).await?;
//! while let Some(frame) = stream.next().await {
//!     let filtered = processor.process(frame).await?;
//!     callback(filtered);
//! }
//! ```
//!
//! For batch processing:
//! ```ignore
//! let filtered = reduce_noise_async(audio_bytes, 16000).await?;
//! ```

use bytes::Bytes;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::LazyLock;
use tokio::sync::{Mutex, mpsc, oneshot};
use tracing::info;

use super::{NoiseFilter, NoiseFilterConfig};

/// A task for a worker: audio bytes, sample rate, and a one-shot channel to send the result back.
type WorkerTask = (
    Bytes,
    u32, // sample_rate
    oneshot::Sender<Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>>,
);

// Type aliases for cleaner code
type WorkerSender = mpsc::Sender<WorkerTask>;
type PoolSender = mpsc::Sender<WorkerSender>;
type PoolReceiver = std::sync::Arc<Mutex<mpsc::Receiver<WorkerSender>>>;
type SenderPool = (PoolSender, PoolReceiver);

/// Get the cache path from environment variable, if set.
fn get_cache_path() -> Option<PathBuf> {
    std::env::var("CACHE_PATH").ok().map(PathBuf::from)
}

/// The pool of senders to the worker threads, managed by a channel.
/// This is the core of our thread-safe, async-friendly pool for the !Send models.
static SENDER_POOL: LazyLock<SenderPool> = LazyLock::new(|| {
    // Use a pool size based on available CPU cores, with a minimum of 2.
    let pool_size = num_cpus::get().max(2);
    let (pool_tx, pool_rx) = mpsc::channel(pool_size);

    for _ in 0..pool_size {
        let pool_tx_clone = pool_tx.clone();

        // Spawn a dedicated OS thread for each worker.
        // This is crucial for pinning the !Send ORT session to a single thread.
        std::thread::spawn(move || {
            let (worker_task_tx, mut worker_task_rx) = mpsc::channel::<WorkerTask>(1);

            // Provide this worker's sender to the pool so it can be checked out.
            if pool_tx_clone.blocking_send(worker_task_tx).is_err() {
                // The pool has been dropped, so this worker is not needed.
                return;
            }

            // Create tokio runtime for this thread to run async initialization
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to create tokio runtime for worker thread");

            // Build config with cache path from environment
            let cache_path = get_cache_path();
            // Note: atten_lim_db is set to 15.0 for STT-optimized processing.
            // 15dB = ~18% signal floor, preserving speech harmonics for accurate transcription.
            // Higher values (40dB) cause over-attenuation and ~20% WER degradation for STT.
            // Post-filter is disabled (0.0) as it causes additional over-attenuation.
            // See: https://github.com/Rikorose/DeepFilterNet/issues/483
            let config = NoiseFilterConfig {
                cache_path,
                atten_lim_db: 15.0, // Conservative limit for STT quality (vs 40dB for aggressive filtering)
                post_filter_beta: 0.0, // Disabled - causes over-attenuation harmful to STT
                ..Default::default()
            };

            // Create NoiseFilter using async/await within the blocking thread
            let filter_result = rt.block_on(NoiseFilter::new(config));
            let mut filter = match filter_result {
                Ok(f) => f,
                Err(e) => {
                    tracing::error!("Failed to create NoiseFilter in worker thread: {}", e);
                    return;
                }
            };

            info!(
                "DeepFilterNet ORT worker initialized with STT-optimized settings (atten_lim=15dB, no post-filter)"
            );

            // The worker's main loop. It blocks efficiently until a task arrives.
            while let Some((pcm, sample_rate, result_tx)) = worker_task_rx.blocking_recv() {
                // Check if sample rate has changed and re-initialize filter if needed
                if filter.config().sample_rate != sample_rate {
                    tracing::debug!(
                        "Sample rate changed from {} to {}, re-initializing filter",
                        filter.config().sample_rate,
                        sample_rate
                    );
                    let new_config = NoiseFilterConfig {
                        sample_rate,
                        ..filter.config().clone()
                    };
                    match rt.block_on(NoiseFilter::new(new_config)) {
                        Ok(new_filter) => {
                            filter = new_filter;
                        }
                        Err(e) => {
                            tracing::error!(
                                "Failed to re-initialize NoiseFilter with sample rate {}: {}",
                                sample_rate,
                                e
                            );
                            // Send back original audio on error
                            let _ = result_tx.send(Ok(pcm.to_vec()));
                            continue;
                        }
                    }
                }

                // Use catch_unwind to prevent panics from killing the worker thread.
                // This is critical for pool stability - a panic in processing shouldn't
                // kill the entire worker, just fail that one request.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    process_audio_chunk(&mut filter, &pcm, sample_rate)
                }));

                let send_result = match result {
                    Ok(Ok(processed)) => Ok(processed),
                    Ok(Err(e)) => {
                        tracing::warn!("Noise filter processing error: {}", e);
                        // Return original audio on processing error
                        Ok(pcm.to_vec())
                    }
                    Err(panic_info) => {
                        // A panic occurred - log it and reset the filter state
                        let panic_msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                            s.to_string()
                        } else if let Some(s) = panic_info.downcast_ref::<String>() {
                            s.clone()
                        } else {
                            "Unknown panic".to_string()
                        };
                        tracing::error!(
                            "Noise filter panicked (recovered): {}. Resetting filter state.",
                            panic_msg
                        );

                        // Reset filter state to recover from any corruption
                        filter.reset();

                        // Return original audio on panic
                        Ok(pcm.to_vec())
                    }
                };

                // Send result back to the caller. Ignore error if caller hung up.
                let _ = result_tx.send(send_result);
            }
        });
    }

    (pool_tx, std::sync::Arc::new(Mutex::new(pool_rx)))
});

/// RAII guard that holds a worker sender checked out from the pool.
/// On drop, it automatically returns the sender to the pool for reuse.
struct PooledWorkerSender {
    sender: Option<mpsc::Sender<WorkerTask>>,
    pool_tx: mpsc::Sender<mpsc::Sender<WorkerTask>>,
}

impl Drop for PooledWorkerSender {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            // Best-effort attempt to return the sender.
            // If the pool is full or closed, the sender is dropped, and the worker will eventually exit.
            let _ = self.pool_tx.try_send(sender);
        }
    }
}

impl Deref for PooledWorkerSender {
    type Target = mpsc::Sender<WorkerTask>;
    fn deref(&self) -> &Self::Target {
        self.sender.as_ref().expect("Sender should always be Some")
    }
}

/// Asynchronously checks out a worker sender from the pool.
/// It will wait efficiently if the pool is empty.
async fn get_sender() -> PooledWorkerSender {
    let (pool_tx, pool_rx_mutex) = &*SENDER_POOL;
    let sender = {
        // Lock the mutex asynchronously. The guard is Send and can be held across an await.
        let mut pool_rx = pool_rx_mutex.lock().await;
        pool_rx
            .recv()
            .await
            .expect("Worker pool channel closed unexpectedly. All worker threads may have died.")
    };

    PooledWorkerSender {
        sender: Some(sender),
        pool_tx: pool_tx.clone(),
    }
}

/// Processes audio chunk using the ORT-based NoiseFilter.
///
/// # Arguments
/// * `filter` - Mutable reference to the NoiseFilter
/// * `pcm` - Raw PCM audio data (16-bit little-endian format)
/// * `sample_rate` - Audio sample rate in Hz (e.g., 16000, 48000)
///
/// # Returns
/// Processed audio as PCM bytes, or original audio if processing fails
#[inline]
fn process_audio_chunk(
    filter: &mut NoiseFilter,
    pcm: &[u8],
    _sample_rate: u32,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    // Early validation to avoid unnecessary processing
    let pcm_len = pcm.len();
    if pcm_len < 2 || !pcm_len.is_multiple_of(2) {
        return Ok(pcm.to_owned());
    }

    // Convert PCM bytes to i16 samples
    let sample_count = pcm_len / 2;
    let mut samples = Vec::with_capacity(sample_count);
    for chunk in pcm.chunks_exact(2) {
        let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
        samples.push(sample);
    }

    // Early return for empty audio
    if samples.is_empty() {
        return Ok(pcm.to_owned());
    }

    // Process through NoiseFilter
    let filtered = match filter.process(&samples) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("Noise filter processing failed: {}", e);
            return Ok(pcm.to_owned());
        }
    };

    // Convert back to PCM bytes
    let mut result = Vec::with_capacity(filtered.len() * 2);
    for sample in filtered {
        result.extend_from_slice(&sample.to_le_bytes());
    }

    Ok(result)
}

/// Performs asynchronous noise reduction using a worker thread pool.
///
/// This function provides the main public interface for noise reduction.
/// It efficiently manages CPU-intensive processing across multiple threads
/// while maintaining an async interface for the caller.
///
/// The function uses a pool of worker threads, each with its own NoiseFilter
/// instance, to process audio in parallel without blocking the async runtime.
///
/// **Note:** For real-time streaming audio, use `StreamNoiseProcessor` instead
/// to maintain buffer state continuity across frames.
///
/// # Arguments
/// * `pcm` - Audio data in PCM format (16-bit signed integer, little-endian)
/// * `sample_rate` - Sample rate of the audio in Hz (common values: 16000, 48000)
///
/// # Returns
/// * `Ok(Vec<u8>)` - Processed audio data with reduced noise
/// * `Err` - If processing fails or worker threads are unavailable
///
/// # Example
/// ```ignore
/// let processed = reduce_noise_async(audio_bytes, 16000).await?;
/// ```
#[inline]
pub async fn reduce_noise_async(
    pcm: Bytes,
    sample_rate: u32,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    // 1. Asynchronously get a sender to a free worker.
    let sender = get_sender().await;

    // 2. Create a one-shot channel to receive the result.
    let (response_tx, response_rx) = oneshot::channel();

    // 3. Send the task to the worker. If it fails, the worker thread has likely died.
    sender
        .send((pcm, sample_rate, response_tx))
        .await
        .map_err(|_| "Failed to send task to worker thread.")?;

    // 4. Await the result. The sender is returned to the pool automatically when it's dropped.
    response_rx
        .await
        .map_err(|_| "Worker thread panicked or channel closed while processing.")?
}

/// Per-stream noise processor for real-time audio streaming.
///
/// Unlike the worker pool (`reduce_noise_async`), this processor maintains a dedicated
/// `NoiseFilter` instance for a single audio stream. This ensures proper buffer state
/// continuity across consecutive audio frames, which is essential for real-time streaming.
///
/// # Usage
///
/// Create one `StreamNoiseProcessor` per audio stream (e.g., per LiveKit participant):
///
/// ```ignore
/// let processor = StreamNoiseProcessor::new(16000).await?;
///
/// while let Some(audio_frame) = stream.next().await {
///     let filtered = processor.process(audio_frame, 16000).await?;
///     callback(filtered);
/// }
/// ```
pub struct StreamNoiseProcessor {
    /// Channel to send audio to the dedicated worker thread.
    task_tx: mpsc::Sender<WorkerTask>,
    /// Handle to the dedicated worker thread (for cleanup).
    _thread_handle: std::thread::JoinHandle<()>,
}

impl StreamNoiseProcessor {
    /// Create a new per-stream noise processor.
    ///
    /// Spawns a dedicated thread with its own `NoiseFilter` instance.
    pub async fn new(sample_rate: u32) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let (task_tx, task_rx) = mpsc::channel::<WorkerTask>(8);
        let (ready_tx, ready_rx) = oneshot::channel::<Result<(), String>>();

        let thread_handle = std::thread::spawn(move || {
            Self::worker_thread(task_rx, ready_tx, sample_rate);
        });

        // Wait for the worker to initialize
        ready_rx
            .await
            .map_err(|_| "Worker thread failed to start")??;

        Ok(Self {
            task_tx,
            _thread_handle: thread_handle,
        })
    }

    /// Worker thread that owns the NoiseFilter and processes audio sequentially.
    fn worker_thread(
        mut task_rx: mpsc::Receiver<WorkerTask>,
        ready_tx: oneshot::Sender<Result<(), String>>,
        sample_rate: u32,
    ) {
        // Create tokio runtime for async operations
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                let _ = ready_tx.send(Err(format!("Failed to create tokio runtime: {}", e)));
                return;
            }
        };

        // Build config with cache path from environment
        // STT-optimized: 15dB limit preserves speech, post-filter disabled
        let cache_path = get_cache_path();
        let config = NoiseFilterConfig {
            cache_path,
            sample_rate,
            atten_lim_db: 15.0, // Conservative for STT quality
            post_filter_beta: 0.0, // Disabled - causes over-attenuation
            ..Default::default()
        };

        // Create NoiseFilter
        let mut filter = match rt.block_on(NoiseFilter::new(config)) {
            Ok(f) => f,
            Err(e) => {
                let _ = ready_tx.send(Err(format!("Failed to create NoiseFilter: {}", e)));
                return;
            }
        };

        info!(
            "StreamNoiseProcessor worker initialized (sample_rate={}Hz)",
            sample_rate
        );

        // Signal that we're ready
        if ready_tx.send(Ok(())).is_err() {
            return;
        }

        // Process tasks sequentially
        while let Some((pcm, req_sample_rate, result_tx)) = task_rx.blocking_recv() {
            // Check if sample rate changed (shouldn't happen in normal use)
            if filter.config().sample_rate != req_sample_rate {
                tracing::warn!(
                    "Sample rate mismatch in stream processor: expected {}, got {}",
                    filter.config().sample_rate,
                    req_sample_rate
                );
            }

            // Process with panic recovery
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                process_audio_chunk(&mut filter, &pcm, req_sample_rate)
            }));

            let send_result = match result {
                Ok(Ok(processed)) => Ok(processed),
                Ok(Err(e)) => {
                    tracing::warn!("Stream noise filter error: {}", e);
                    Ok(pcm.to_vec())
                }
                Err(panic_info) => {
                    let panic_msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                        s.to_string()
                    } else if let Some(s) = panic_info.downcast_ref::<String>() {
                        s.clone()
                    } else {
                        "Unknown panic".to_string()
                    };
                    tracing::error!(
                        "Stream noise filter panicked (recovered): {}. Resetting filter.",
                        panic_msg
                    );
                    filter.reset();
                    Ok(pcm.to_vec())
                }
            };

            let _ = result_tx.send(send_result);
        }

        info!("StreamNoiseProcessor worker shutting down");
    }

    /// Process audio through the dedicated noise filter.
    ///
    /// This method sends the audio to the dedicated worker thread and waits for
    /// the filtered result. Unlike the pool-based approach, consecutive calls
    /// are guaranteed to be processed by the same filter instance.
    pub async fn process(
        &self,
        pcm: Bytes,
        sample_rate: u32,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let (response_tx, response_rx) = oneshot::channel();

        self.task_tx
            .send((pcm, sample_rate, response_tx))
            .await
            .map_err(|_| "Stream processor worker has stopped")?;

        response_rx
            .await
            .map_err(|_| "Stream processor worker crashed")?
    }
}

/// Stub version of StreamNoiseProcessor for when noise-filter feature is disabled.
/// Passes audio through unchanged.
#[cfg(not(feature = "noise-filter"))]
pub struct StreamNoiseProcessorStub;

#[cfg(not(feature = "noise-filter"))]
impl StreamNoiseProcessorStub {
    pub async fn new(_sample_rate: u32) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Self)
    }

    pub async fn process(
        &self,
        pcm: Bytes,
        _sample_rate: u32,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(pcm.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_pcm(samples: usize) -> Vec<u8> {
        let mut pcm = Vec::with_capacity(samples * 2);
        for i in 0..samples {
            let value = ((i as f32 / samples as f32) * 1_000.0).sin();
            let sample = (value * 16_384.0) as i16;
            pcm.extend_from_slice(&sample.to_le_bytes());
        }
        pcm
    }

    /// Helper to check if noise filter models are available.
    fn models_available() -> bool {
        if let Ok(cache_path) = std::env::var("CACHE_PATH") {
            let noise_filter_dir = std::path::Path::new(&cache_path).join("noise_filter");
            noise_filter_dir.join("enc.onnx").exists()
                && noise_filter_dir.join("erb_dec.onnx").exists()
                && noise_filter_dir.join("df_dec.onnx").exists()
        } else {
            false
        }
    }

    #[tokio::test]
    async fn test_silent_audio_passthrough() {
        if !models_available() {
            eprintln!("Skipping test: noise filter models not available");
            return;
        }

        let silent_pcm = vec![0u8; 320];
        let result = reduce_noise_async(Bytes::from(silent_pcm.clone()), 16_000)
            .await
            .expect("Processing should succeed");
        assert_eq!(result, silent_pcm);
    }

    #[tokio::test]
    async fn test_normal_audio_processing() {
        if !models_available() {
            eprintln!("Skipping test: noise filter models not available");
            return;
        }

        let pcm = create_test_pcm(16_000);
        let result = reduce_noise_async(Bytes::from(pcm.clone()), 16_000)
            .await
            .expect("Processing should succeed");
        assert!(!result.is_empty());
    }

    #[tokio::test]
    async fn test_stream_processor() {
        if !models_available() {
            eprintln!("Skipping test: noise filter models not available");
            return;
        }

        let processor = StreamNoiseProcessor::new(16_000)
            .await
            .expect("Should create processor");

        // Process multiple frames through the same processor
        for _ in 0..5 {
            let pcm = create_test_pcm(320); // 20ms @ 16kHz
            let result = processor
                .process(Bytes::from(pcm), 16_000)
                .await
                .expect("Processing should succeed");
            // Result may be empty during initial buffering
            assert!(result.len() <= 640 * 2);
        }
    }
}
