use bytes::Bytes;
use deep_filter::tract::{DfParams, DfTract, RuntimeParams};
use ndarray::{Array2, ArrayView2};
use std::cell::RefCell;
use tokio::task;

/// Thread-safe wrapper for DeepFilter model parameters
/// Since DfTract is not Send/Sync, we cache instances per thread
///
/// This configuration enables post-filtering and optimizes parameters for better
/// noise cancellation, especially for echo and reverb scenarios.
static MODEL_PARAMS: std::sync::LazyLock<(DfParams, RuntimeParams)> =
    std::sync::LazyLock::new(|| {
        let df_params = DfParams::default();
        let mut rt_params = RuntimeParams::default();

        // Enable post-filter for better noise suppression, especially for reverb/echo
        rt_params.post_filter = true; // Enable post-filtering
        rt_params.post_filter_beta = 0.02; // Post-filter strength (similar to Python --pf option)

        // Optimize for better echo/reverb suppression
        rt_params.atten_lim_db = 100.0; // Maximum attenuation limit
        rt_params.min_db_thresh = -10.0; // Minimum processing threshold

        (df_params, rt_params)
    });

// Thread-local storage for cached model instances
// Each thread gets its own cached DfTract instance to avoid recreation overhead
thread_local! {
    static THREAD_MODEL: RefCell<Option<DfTract>> = RefCell::new(None);
}

/// Get or create a model instance for the current thread
fn get_or_create_model() -> Result<DfTract, Box<dyn std::error::Error + Send + Sync>> {
    THREAD_MODEL.with(|model_cell| {
        let mut model_opt = model_cell.borrow_mut();
        if let Some(model) = model_opt.take() {
            // Reuse existing model from thread-local storage
            Ok(model)
        } else {
            // Create new model for this thread
            let (df_params, rt_params) = &*MODEL_PARAMS;
            Ok(DfTract::new(df_params.clone(), rt_params)?)
        }
    })
}

/// Return a model instance to thread-local storage for reuse
fn return_model_to_thread(model: DfTract) {
    THREAD_MODEL.with(|model_cell| {
        *model_cell.borrow_mut() = Some(model);
    });
}

/// PCM i16 -> PCM i16 noise reduction
pub fn reduce_noise_blocking(
    pcm: &[u8],
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    // 1) convert & normalize - use consistent normalization with Python implementation
    let wav: Vec<f32> = pcm
        .chunks_exact(2)
        .map(|b| {
            let sample = i16::from_le_bytes([b[0], b[1]]) as f32;
            // Use consistent normalization: divide by 32768.0 and clamp to [-1.0, 1.0]
            (sample / 32768.0).clamp(-1.0, 1.0)
        })
        .collect();

    // trivial silence check
    if wav.iter().all(|s| s.abs() < 1e-7) {
        return Ok(pcm.to_owned());
    }

    // Get or create a cached model instance for this thread
    let mut df = get_or_create_model()?;
    let hop = df.hop_size; // This is a field, not a method

    // 2) run the real-time loop expected by DeepFilterNet:
    //    feed hop-sized fragments and collect the enhanced samples.
    let mut enhanced = Vec::with_capacity(wav.len());
    let mut idx = 0;

    // Process in hop-sized chunks
    while idx + hop <= wav.len() {
        // Create frame using deep_filter's ndarray version
        let frame = ArrayView2::from_shape((1, hop), &wav[idx..idx + hop])?;
        let mut out = Array2::<f32>::zeros((1, hop));
        df.process(frame, out.view_mut())?;
        enhanced.extend_from_slice(out.as_slice().unwrap());
        idx += hop;
    }

    // Handle remaining samples with zero-padding if needed
    let remaining = wav.len() - idx;
    if remaining > 0 {
        // For remaining samples, we need to zero-pad to hop size for processing
        if remaining >= hop / 2 {
            // Process if we have at least half a frame
            let mut padded_frame = vec![0.0f32; hop];
            padded_frame[..remaining].copy_from_slice(&wav[idx..]);

            let frame = ArrayView2::from_shape((1, hop), &padded_frame)?;
            let mut out = Array2::<f32>::zeros((1, hop));
            df.process(frame, out.view_mut())?;

            // Only take the actual samples, not the padding
            enhanced.extend_from_slice(&out.as_slice().unwrap()[..remaining]);
        } else {
            // For very small remaining segments, just copy as-is
            enhanced.extend_from_slice(&wav[idx..]);
        }
    }

    // Return the model instance to thread-local storage for reuse
    return_model_to_thread(df);

    // 3) convert back to i16 bytes with proper denormalization
    let result = enhanced
        .iter()
        .map(|s| {
            // Clamp to valid range first, then scale and convert
            let clamped = s.clamp(-1.0, 1.0);
            // Use symmetric scaling for better dynamic range
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

/// Async wrapper – *this* is what you call from Tokio code.
pub async fn reduce_noise_async(
    pcm: Bytes,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    // `spawn_blocking` runs the closure on Tokio’s blocking thread-pool
    let result = task::spawn_blocking(move || reduce_noise_blocking(&pcm))
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
    result
}
