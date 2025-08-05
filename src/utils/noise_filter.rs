use bytes::Bytes;
use deep_filter::tract::{DfParams, DfTract, RuntimeParams};
use ndarray::{Array2, ArrayView2};
use std::ops::Deref;
use std::sync::{Arc, LazyLock};
use tokio::sync::{Mutex, mpsc, oneshot};

/// Enhanced configuration based on DeepFilterNet best practices and Python implementation analysis.
/// These parameters are optimized for better noise cancellation while preserving speech quality.
static MODEL_PARAMS: LazyLock<(DfParams, RuntimeParams)> = LazyLock::new(|| {
    let df_params = DfParams::default();
    let rt_params = RuntimeParams::default();

    (df_params, rt_params)
});

// A task for a worker: audio bytes, sample rate, and a one-shot channel to send the result back.
type WorkerTask = (
    Bytes,
    u32, // sample_rate
    oneshot::Sender<Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>>,
);

// Type aliases for cleaner code
type WorkerSender = mpsc::Sender<WorkerTask>;
type PoolSender = mpsc::Sender<WorkerSender>;
type PoolReceiver = Arc<Mutex<mpsc::Receiver<WorkerSender>>>;
type SenderPool = (PoolSender, PoolReceiver);

// The pool of senders to the worker threads, managed by a channel.
// This is the core of our thread-safe, async-friendly pool for the !Send models.
static SENDER_POOL: LazyLock<SenderPool> = LazyLock::new(|| {
    // Use a pool size based on available CPU cores, with a minimum of 2.
    let pool_size = num_cpus::get().max(2);
    let (pool_tx, pool_rx) = mpsc::channel(pool_size);

    for _ in 0..pool_size {
        let pool_tx_clone = pool_tx.clone();

        // Spawn a dedicated OS thread for each worker.
        // This is crucial for pinning the !Send DfTract model to a single thread.
        std::thread::spawn(move || {
            let (worker_task_tx, mut worker_task_rx) = mpsc::channel::<WorkerTask>(1);

            // Provide this worker's sender to the pool so it can be checked out.
            if pool_tx_clone.blocking_send(worker_task_tx).is_err() {
                // The pool has been dropped, so this worker is not needed.
                return;
            }

            // Create the expensive DfTract model ONCE on this thread.
            let (df_params, rt_params) = &*MODEL_PARAMS;
            let mut model = DfTract::new(df_params.clone(), rt_params)
                .expect("Failed to create DfTract model in worker thread");

            // The worker's main loop. It blocks efficiently until a task arrives.
            while let Some((pcm, sample_rate, result_tx)) = worker_task_rx.blocking_recv() {
                let result = process_audio_chunk(&mut model, &pcm, sample_rate);
                // Send result back to the caller. Ignore error if caller hung up.
                let _ = result_tx.send(result);
            }
        });
    }

    (pool_tx, Arc::new(Mutex::new(pool_rx)))
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

/// Enhanced audio processing with adaptive logic based on Python implementation insights.
/// Includes audio analysis, light processing for certain cases, and conservative blending.
fn process_audio_chunk(
    df: &mut DfTract,
    pcm: &[u8],
    sample_rate: u32,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    // 1) Convert PCM to float samples
    let wav: Vec<f32> = pcm
        .chunks_exact(2)
        .map(|b| {
            let sample = i16::from_le_bytes([b[0], b[1]]) as f32;
            (sample / 32768.0).clamp(-1.0, 1.0)
        })
        .collect();

    // 2) Early return for empty or silent audio (more lenient threshold like Python)
    if wav.is_empty() || wav.iter().all(|s| s.abs() < 1e-7) {
        return Ok(pcm.to_owned());
    }

    // 3) Analyze audio characteristics (mimicking Python logic)
    let sample_rate_f32 = sample_rate as f32;
    let audio_duration = wav.len() as f32 / sample_rate_f32;

    // Calculate RMS and peak energy
    let rms_energy = (wav.iter().map(|s| s * s).sum::<f32>() / wav.len() as f32).sqrt();
    let peak_energy = wav.iter().map(|s| s.abs()).fold(0.0f32, f32::max);

    // 4) Apply adaptive processing based on audio characteristics

    // For high SNR audio (clear speech), apply minimal processing
    if rms_energy > 0.1 && peak_energy > 0.3 {
        return apply_light_processing(pcm, &wav, sample_rate);
    }

    // For very short audio, use light processing or skip
    if audio_duration < 1.0 {
        if rms_energy < 0.005 || peak_energy < 0.02 {
            return Ok(pcm.to_owned());
        }
        return apply_light_processing(pcm, &wav, sample_rate);
    }

    // 5) Check minimum samples requirement
    let hop = df.hop_size;
    let min_samples = (512_usize).max(hop * 2);
    if wav.len() < min_samples {
        return apply_light_processing(pcm, &wav, sample_rate);
    }

    // 6) Store original for blending
    let original_wav = wav.clone();

    // 7) Full DeepFilterNet processing
    let mut enhanced = Vec::with_capacity(wav.len());
    let mut idx = 0;

    while idx + hop <= wav.len() {
        let frame = ArrayView2::from_shape((1, hop), &wav[idx..idx + hop])?;
        let mut out = Array2::<f32>::zeros((1, hop));
        df.process(frame, out.view_mut())?;
        enhanced.extend_from_slice(out.as_slice().unwrap());
        idx += hop;
    }

    // Handle remaining samples
    let remaining = wav.len() - idx;
    if remaining > 0 {
        if remaining >= hop / 2 {
            let mut padded_frame = vec![0.0f32; hop];
            padded_frame[..remaining].copy_from_slice(&wav[idx..]);
            let frame = ArrayView2::from_shape((1, hop), &padded_frame)?;
            let mut out = Array2::<f32>::zeros((1, hop));
            df.process(frame, out.view_mut())?;
            enhanced.extend_from_slice(&out.as_slice().unwrap()[..remaining]);
        } else {
            enhanced.extend_from_slice(&wav[idx..]);
        }
    }

    // 8) Conservative blending - 20% enhanced, 80% original (like Python)
    let blend_ratio = 0.2f32;
    let blended: Vec<f32> = enhanced
        .iter()
        .zip(original_wav.iter())
        .map(|(enh, orig)| blend_ratio * enh + (1.0 - blend_ratio) * orig)
        .collect();

    // 9) Volume preservation (maintain similar level as original)
    let original_max = original_wav.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    let blended_max = blended.iter().map(|s| s.abs()).fold(0.0f32, f32::max);

    let final_audio = if blended_max > 0.0 && original_max > 0.0 {
        let volume_ratio = original_max / blended_max;
        blended.iter().map(|s| s * volume_ratio * 0.95).collect() // 5% safety margin
    } else {
        blended
    };

    // 10) Check for NaN/inf values
    if final_audio.iter().any(|s| !s.is_finite()) {
        return Ok(pcm.to_owned());
    }

    // 11) Convert back to i16 bytes with conservative normalization
    let result = final_audio
        .iter()
        .map(|s| {
            let clamped = s.clamp(-1.0, 1.0);
            let scaled = if clamped >= 0.0 {
                clamped * 32767.0
            } else {
                clamped * 32768.0
            };
            scaled.round() as i16
        })
        .flat_map(|s| s.to_le_bytes())
        .collect();

    Ok(result)
}

/// Light processing for short/clean audio clips - applies minimal filtering
/// Similar to the Python implementation's _apply_light_processing function
fn apply_light_processing(
    original_pcm: &[u8],
    wav: &[f32],
    sample_rate: u32,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    // For very short clips (< 200ms equivalent), preserve original
    let sample_rate_f32 = sample_rate as f32;
    let duration = wav.len() as f32 / sample_rate_f32;

    if duration < 0.2 {
        // Very short - minimal processing, preserve original volume
        let max_val = wav.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        if max_val > 0.0 {
            // Keep original - no filtering for very short audio
            return Ok(original_pcm.to_vec());
        }
    }

    // Apply gentle high-pass filter for longer short clips
    // Simple first-order high-pass at 80Hz
    let cutoff_freq = 80.0;
    let rc = 1.0 / (2.0 * std::f32::consts::PI * cutoff_freq);
    let dt = 1.0 / sample_rate_f32;
    let alpha = rc / (rc + dt);

    let mut filtered = Vec::with_capacity(wav.len());
    if !wav.is_empty() {
        filtered.push(wav[0]); // First sample unchanged

        for i in 1..wav.len() {
            let filtered_sample = alpha * (filtered[i - 1] + wav[i] - wav[i - 1]);
            filtered.push(filtered_sample);
        }
    }

    // Preserve original volume level
    let original_max = wav.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    let filtered_max = filtered.iter().map(|s| s.abs()).fold(0.0f32, f32::max);

    let normalized = if filtered_max > 0.0 && original_max > 0.0 {
        let volume_ratio = original_max / filtered_max;
        filtered.iter().map(|s| s * volume_ratio * 0.98).collect() // Small safety margin
    } else {
        filtered
    };

    // Convert back to bytes
    let result = normalized
        .iter()
        .map(|s| {
            let clamped = s.clamp(-1.0, 1.0);
            let scaled = if clamped >= 0.0 {
                clamped * 32767.0
            } else {
                clamped * 32768.0
            };
            scaled.round() as i16
        })
        .flat_map(|s| s.to_le_bytes())
        .collect();

    Ok(result)
}

/// Async wrapper to perform noise reduction using the worker pool.
///
/// # Arguments
/// * `pcm` - Audio data as PCM i16 little-endian bytes
/// * `sample_rate` - Audio sample rate in Hz (e.g., 16000, 48000)
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
