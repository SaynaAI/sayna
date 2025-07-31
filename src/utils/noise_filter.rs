use bytes::Bytes;
use deep_filter::tract::{DfParams, DfTract, RuntimeParams};
use ndarray::{Array2, ArrayView2};
use std::ops::Deref;
use std::sync::{Arc, LazyLock};
use tokio::sync::{Mutex, mpsc, oneshot};

/// This configuration enables post-filtering and optimizes parameters for better
/// noise cancellation, especially for echo and reverb scenarios.
static MODEL_PARAMS: LazyLock<(DfParams, RuntimeParams)> = LazyLock::new(|| {
    let df_params = DfParams::default();
    let mut rt_params = RuntimeParams::default();
    rt_params.post_filter = true;
    rt_params.post_filter_beta = 0.02;
    rt_params.atten_lim_db = 100.0;
    rt_params.min_db_thresh = -10.0;
    (df_params, rt_params)
});

// A task for a worker: audio bytes and a one-shot channel to send the result back.
type WorkerTask = (
    Bytes,
    oneshot::Sender<Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>>,
);

// The pool of senders to the worker threads, managed by a channel.
// This is the core of our thread-safe, async-friendly pool for the !Send models.
static SENDER_POOL: LazyLock<(
    mpsc::Sender<mpsc::Sender<WorkerTask>>,
    Arc<Mutex<mpsc::Receiver<mpsc::Sender<WorkerTask>>>>,
)> = LazyLock::new(|| {
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
            while let Some((pcm, result_tx)) = worker_task_rx.blocking_recv() {
                let result = process_audio_chunk(&mut model, &pcm);
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

/// The core noise reduction logic, now extracted to be run by a worker.
fn process_audio_chunk(
    df: &mut DfTract,
    pcm: &[u8],
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    // 1) convert & normalize
    let wav: Vec<f32> = pcm
        .chunks_exact(2)
        .map(|b| {
            let sample = i16::from_le_bytes([b[0], b[1]]) as f32;
            (sample / 32768.0).clamp(-1.0, 1.0)
        })
        .collect();

    if wav.iter().all(|s| s.abs() < 1e-7) {
        return Ok(pcm.to_owned());
    }

    let hop = df.hop_size;
    let mut enhanced = Vec::with_capacity(wav.len());
    let mut idx = 0;

    // 2) run the real-time loop
    while idx + hop <= wav.len() {
        let frame = ArrayView2::from_shape((1, hop), &wav[idx..idx + hop])?;
        let mut out = Array2::<f32>::zeros((1, hop));
        df.process(frame, out.view_mut())?;
        enhanced.extend_from_slice(out.as_slice().unwrap());
        idx += hop;
    }

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

    // 3) convert back to i16 bytes
    let result = enhanced
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
pub async fn reduce_noise_async(
    pcm: Bytes,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    // 1. Asynchronously get a sender to a free worker.
    let sender = get_sender().await;

    // 2. Create a one-shot channel to receive the result.
    let (response_tx, response_rx) = oneshot::channel();

    // 3. Send the task to the worker. If it fails, the worker thread has likely died.
    sender
        .send((pcm, response_tx))
        .await
        .map_err(|_| "Failed to send task to worker thread.")?;

    // 4. Await the result. The sender is returned to the pool automatically when it's dropped.
    response_rx
        .await
        .map_err(|_| "Worker thread panicked or channel closed while processing.")?
}
