use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures_util::StreamExt;
use tokio::sync::{Mutex, Notify, RwLock, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use super::base::{AudioCallback, AudioData, ConnectionState, TTSConfig, TTSError, TTSResult};
use crate::core::cache::store::CacheStore;
use crate::utils::req_manager::{ReqManager, ReqManagerConfig};
use regex::Regex;
use std::time::Duration;
use xxhash_rust::xxh3::xxh3_128;

/// Request entry for ordered processing
struct RequestEntry {
    receiver: mpsc::Receiver<Result<Vec<u8>, TTSError>>,
    /// Audio format for this specific request
    format: String,
    /// Sample rate for this specific request
    sample_rate: u32,
}

/// Work item representing a single speak job to be processed sequentially
struct WorkItem {
    /// Future that executes the send_request call
    task: Pin<Box<dyn Future<Output = ()> + Send>>,
    /// Request entry to be added to pending_queue before executing task
    entry: RequestEntry,
}

/// Compiled pronunciation replacement patterns
#[derive(Clone)]
pub struct PronunciationReplacer {
    patterns: Vec<(Regex, String)>,
}

impl PronunciationReplacer {
    /// Create a new pronunciation replacer from config
    pub fn new(pronunciations: &[super::base::Pronunciation]) -> Self {
        let patterns = pronunciations
            .iter()
            .filter_map(|p| {
                // Create word-boundary aware regex for each pronunciation
                let pattern = format!(r"\b{}\b", regex::escape(&p.word));
                match Regex::new(&pattern) {
                    Ok(regex) => Some((regex, p.pronunciation.clone())),
                    Err(e) => {
                        error!(
                            "Failed to compile pronunciation pattern for '{}': {}",
                            p.word, e
                        );
                        None
                    }
                }
            })
            .collect();
        Self { patterns }
    }

    /// Apply all pronunciation replacements to text
    pub fn apply(&self, text: &str) -> String {
        let mut result = text.to_string();
        for (pattern, replacement) in &self.patterns {
            result = pattern
                .replace_all(&result, replacement.as_str())
                .into_owned();
        }
        result
    }
}

/// Trait for creating HTTP requests for TTS providers
pub trait TTSRequestBuilder: Send + Sync {
    /// Build the HTTP request with provider-specific URL, headers and body
    /// This is the only provider-specific part that needs to be implemented
    ///
    /// # Arguments
    /// * `client` - The HTTP client to use for building the request
    /// * `text` - The text to synthesize
    ///
    /// # Returns
    /// A request builder ready to be sent
    fn build_http_request(&self, client: &reqwest::Client, text: &str) -> reqwest::RequestBuilder;

    /// Build the HTTP request with additional context (e.g., previous text)
    /// Default implementation falls back to build_http_request without context
    ///
    /// # Arguments
    /// * `client` - The HTTP client to use for building the request
    /// * `text` - The text to synthesize
    /// * `previous_text` - Optional previous text for context continuity
    ///
    /// # Returns
    /// A request builder ready to be sent
    fn build_http_request_with_context(
        &self,
        client: &reqwest::Client,
        text: &str,
        previous_text: Option<&str>,
    ) -> reqwest::RequestBuilder {
        // Default implementation ignores context
        let _ = previous_text;
        self.build_http_request(client, text)
    }

    /// Get the configuration for this request builder
    fn get_config(&self) -> &TTSConfig;

    /// Get precompiled pronunciation replacer
    fn get_pronunciation_replacer(&self) -> Option<&PronunciationReplacer> {
        None
    }
}

/// Generic HTTP-based TTS provider implementation using ReqManager
pub struct TTSProvider {
    /// Connection state
    connected: Arc<AtomicBool>,
    /// Audio callback for handling audio data
    audio_callback: Arc<RwLock<Option<Arc<dyn AudioCallback>>>>,
    /// Ordered queue of pending requests (preserves speak() arrival order)
    pending_queue: Arc<Mutex<VecDeque<RequestEntry>>>,
    /// Request manager for HTTP connections
    req_manager: Arc<RwLock<Option<Arc<ReqManager>>>>,
    /// Job queue for sequential work processing
    job_queue: Arc<Mutex<VecDeque<WorkItem>>>,
    /// Notification for worker when new jobs are available
    job_notify: Arc<Notify>,
    /// Notification for dispatcher when new requests are available
    pending_notify: Arc<Notify>,
    /// Sequential worker task that processes jobs one at a time
    worker_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// Dispatcher task that delivers audio in-order to the callback
    dispatcher_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// Cancellation token that controls current generation lifecycle
    cancel_token: CancellationToken,
    /// Optional cache store for TTS audio
    cache: Arc<RwLock<Option<Arc<CacheStore>>>>,
    /// Precomputed hash for the TTS configuration (stable for provider lifetime)
    tts_config_hash: Arc<RwLock<Option<String>>>,
    /// Previous text for context continuity (session-scoped)
    previous_text: Arc<RwLock<Option<String>>>,
}

impl TTSProvider {
    /// Create a new HTTP-based TTS provider instance
    pub fn new() -> TTSResult<Self> {
        Ok(Self {
            connected: Arc::new(AtomicBool::new(false)),
            audio_callback: Arc::new(RwLock::new(None)),
            pending_queue: Arc::new(Mutex::new(VecDeque::new())),
            req_manager: Arc::new(RwLock::new(None)),
            job_queue: Arc::new(Mutex::new(VecDeque::new())),
            job_notify: Arc::new(Notify::new()),
            pending_notify: Arc::new(Notify::new()),
            worker_task: Arc::new(Mutex::new(None)),
            dispatcher_task: Arc::new(Mutex::new(None)),
            cancel_token: CancellationToken::new(),
            cache: Arc::new(RwLock::new(None)),
            tts_config_hash: Arc::new(RwLock::new(None)),
            previous_text: Arc::new(RwLock::new(None)),
        })
    }

    /// Generic send_request implementation that handles all the common logic
    async fn send_request<R: TTSRequestBuilder>(
        request_builder: &R,
        req_manager: Arc<ReqManager>,
        text: String,
        sender: mpsc::Sender<Result<Vec<u8>, TTSError>>,
        token: CancellationToken,
        cache_and_key: Option<(Arc<CacheStore>, String)>,
        previous_text_store: Arc<RwLock<Option<String>>>,
    ) {
        if token.is_cancelled() {
            let _ = sender
                .send(Err(TTSError::InternalError("Cancelled".to_string())))
                .await;
            return;
        }

        // Get previous text for context (if available)
        let previous_text = previous_text_store.read().await.clone();

        // Apply pronunciation replacements using precompiled regex patterns
        let processed_text = if let Some(replacer) = request_builder.get_pronunciation_replacer() {
            replacer.apply(&text)
        } else {
            text.clone()
        };

        // Try cache first
        if let Some((cache, key)) = cache_and_key.as_ref() {
            match cache.get(key).await {
                Ok(Some(bytes)) => {
                    debug!(
                        "Cache HIT - Sending cached audio for text: '{}', {} bytes",
                        processed_text,
                        bytes.len()
                    );
                    // Send cached audio as a single chunk
                    debug!(
                        "Sending cached audio through channel (will block until receiver reads)..."
                    );
                    if let Err(e) = sender.send(Ok(bytes.clone().to_vec())).await {
                        error!("Failed to send cached audio through channel: {:?}", e);
                    } else {
                        debug!(
                            "Successfully sent cached audio through channel for text: '{}' (receiver has read it)",
                            processed_text
                        );
                    }
                    return;
                }
                Ok(None) => {
                    debug!("Cache miss for text: '{}'", processed_text);
                }
                Err(e) => {
                    error!("Cache get error: {:?}", e);
                }
            }
        }

        // Acquire a client from the pool
        let client_guard = match req_manager.acquire().await {
            Ok(guard) => guard,
            Err(e) => {
                error!("Failed to acquire HTTP client: {}", e);
                let _ = sender
                    .send(Err(TTSError::NetworkError(format!(
                        "Failed to acquire client: {e}"
                    ))))
                    .await;
                return;
            }
        };

        // Build request with provider-specific URL, headers and body using processed text
        // Pass previous_text for context continuity (ElevenLabs uses this)
        let request = request_builder.build_http_request_with_context(
            client_guard.client(),
            &processed_text,
            previous_text.as_deref(),
        );

        // Send request
        let response_result = request.send().await;
        let config = request_builder.get_config();

        match response_result {
            Ok(response) => {
                if !response.status().is_success() {
                    info!("ERROR Response for text: {}", processed_text);
                    let status = response.status();
                    let error_body = response
                        .text()
                        .await
                        .unwrap_or_else(|_| "Unknown error".to_string());
                    error!("TTS API error ({}): {}", status, error_body);

                    let _ = sender
                        .send(Err(TTSError::ProviderError(format!(
                            "API error ({status}): {error_body}"
                        ))))
                        .await;
                    return;
                }

                // Stream body in chunks
                let encoding = config.audio_format.as_deref().unwrap_or("linear16");
                let sample_rate = config.sample_rate.unwrap_or(24000) as usize;
                let (mut chunk_target_bytes, bytes_per_sample) = match encoding {
                    // Assume mono
                    "linear16" | "pcm" => ((sample_rate / 100) * 2, 2usize), // ~10ms
                    "mulaw" | "ulaw" | "alaw" => ((sample_rate / 100), 1usize),
                    _ => (0usize, 0usize),
                };

                // Guard against tiny sample rates
                if chunk_target_bytes == 0
                    && matches!(encoding, "linear16" | "pcm" | "mulaw" | "ulaw" | "alaw")
                {
                    chunk_target_bytes = (sample_rate.max(100) / 100) * bytes_per_sample.max(1);
                }

                let mut buffer: Vec<u8> = Vec::with_capacity(chunk_target_bytes.max(512));
                // Only allocate full_audio buffer if we're caching
                let mut full_audio: Option<Vec<u8>> = if cache_and_key.is_some() {
                    Some(Vec::new())
                } else {
                    None
                };

                let mut stream = response.bytes_stream();
                while let Some(item) = stream.next().await {
                    if token.is_cancelled() {
                        break;
                    }

                    match item {
                        Ok(bytes) => {
                            let mut incoming = bytes.as_ref();

                            if chunk_target_bytes == 0 {
                                // Non-PCM/containerized formats: forward chunks as-is
                                let chunk_vec = incoming.to_vec();
                                // Only accumulate if caching
                                if let Some(ref mut full) = full_audio {
                                    full.extend_from_slice(&chunk_vec);
                                }
                                debug!(
                                    "Sending audio chunk ({} bytes) - will wait for receiver...",
                                    chunk_vec.len()
                                );
                                let _ = sender.send(Ok(chunk_vec)).await;
                                debug!("Audio chunk sent and received");
                                continue;
                            }

                            // PCM-like formats: aggregate into ~10ms chunks
                            while !incoming.is_empty() {
                                let needed = chunk_target_bytes.saturating_sub(buffer.len());
                                let take = needed.min(incoming.len());
                                buffer.extend_from_slice(&incoming[..take]);
                                incoming = &incoming[take..];

                                if buffer.len() >= chunk_target_bytes {
                                    // Take exactly chunk_target_bytes from buffer, leave any excess
                                    let chunk: Vec<u8> =
                                        buffer.drain(..chunk_target_bytes).collect();
                                    // Only accumulate if caching
                                    if let Some(ref mut full) = full_audio {
                                        full.extend_from_slice(&chunk);
                                    }
                                    debug!(
                                        "Sending PCM chunk ({} bytes) - will wait for receiver...",
                                        chunk.len()
                                    );
                                    let _ = sender.send(Ok(chunk)).await;
                                    debug!("PCM chunk sent and received");
                                }
                            }
                        }
                        Err(e) => {
                            error!("Failed to read audio chunk: {}", e);
                            let _ = sender
                                .send(Err(TTSError::AudioGenerationFailed(format!(
                                    "Failed to read audio: {e}"
                                ))))
                                .await;
                            return;
                        }
                    }
                }

                // Flush remainder
                if !buffer.is_empty() && !token.is_cancelled() {
                    // Only accumulate if caching
                    if let Some(ref mut full) = full_audio {
                        full.extend_from_slice(&buffer);
                    }
                    debug!(
                        "Sending final buffer ({} bytes) - will wait for receiver...",
                        buffer.len()
                    );
                    let _ = sender.send(Ok(buffer)).await;
                    debug!("Final buffer sent and received");
                }

                // Store the full audio in cache if provided
                if let Some(((cache, key), full_audio)) = cache_and_key.zip(full_audio) {
                    match cache.put(key, full_audio).await {
                        Ok(_) => {}
                        Err(e) => error!("Failed to cache TTS audio: {:?}", e),
                    }
                }

                // Update previous_text for context continuity on next request
                // Only update if not cancelled and generation succeeded
                if !token.is_cancelled() {
                    *previous_text_store.write().await = Some(processed_text.clone());
                    debug!(
                        "Updated previous_text for next request: '{}'",
                        processed_text
                    );
                }
            }
            Err(e) => {
                error!("HTTP request failed: {}", e);
                let _ = sender
                    .send(Err(TTSError::NetworkError(format!("Request failed: {e}"))))
                    .await;
            }
        }
    }

    /// Set the request manager for this instance
    pub async fn set_req_manager(&mut self, req_manager: Arc<ReqManager>) {
        *self.req_manager.write().await = Some(req_manager);
    }

    /// Get the request manager
    pub async fn get_req_manager(&self) -> Option<Arc<ReqManager>> {
        self.req_manager.read().await.clone()
    }

    /// Process audio chunks with duration calculation
    pub fn process_audio_chunk(bytes: Vec<u8>, format: &str, sample_rate: u32) -> AudioData {
        let duration_ms = if matches!(format, "linear16" | "pcm" | "mulaw" | "ulaw" | "alaw") {
            // Approximate duration for this chunk based on sample_rate and bytes per sample
            let bytes_per_sample = if matches!(format, "linear16" | "pcm") {
                2
            } else {
                1
            };
            let samples = (bytes.len() / bytes_per_sample) as u32;
            Some((samples * 1000) / sample_rate)
        } else {
            None
        };

        AudioData {
            data: bytes,
            sample_rate,
            format: format.to_string(),
            duration_ms,
        }
    }

    /// Spawn the dispatcher if not already running.
    ///
    /// The dispatcher is responsible for delivering audio to the callback in the
    /// exact order texts were enqueued via `speak`.
    async fn ensure_dispatcher(&self) {
        let mut task_guard = self.dispatcher_task.lock().await;
        if task_guard.is_some() {
            return;
        }

        let pending_queue = self.pending_queue.clone();
        let audio_callback = self.audio_callback.clone();
        let pending_notify = self.pending_notify.clone();
        let token = self.cancel_token.clone();

        let handle = tokio::spawn(async move {
            debug!("TTS dispatcher task started");

            loop {
                tokio::select! {
                    _ = token.cancelled() => {
                        debug!("TTS dispatcher cancelled");
                        break;
                    }
                    _ = async {
                        // Get next request from queue
                        let entry = {
                            let mut queue = pending_queue.lock().await;
                            let size = queue.len();
                            if size > 0 {
                                debug!("TTS dispatcher: {} requests in queue", size);
                            }
                            queue.pop_front()
                        };

                        let Some(mut entry) = entry else {
                            // No pending items, wait for notification
                            pending_notify.notified().await;
                            return;
                        };

                        debug!("TTS dispatcher processing request from queue");

                        // Use the format and sample rate from this specific request
                        let format = entry.format.clone();
                        let sample_rate = entry.sample_rate;

                        // Get callback once for this text
                        let cb_opt = audio_callback.read().await.clone();

                        // Process all chunks from this request's receiver
                        debug!("TTS dispatcher waiting for audio chunks from receiver");
                        let mut chunk_count = 0;
                        while let Some(result) = entry.receiver.recv().await {
                            match result {
                                Ok(bytes) => {
                                    chunk_count += 1;
                                    debug!("TTS dispatcher received chunk #{}: {} bytes", chunk_count, bytes.len());
                                    if let Some(cb) = cb_opt.as_ref() {
                                        let duration_ms = if matches!(
                                            format.as_str(),
                                            "linear16" | "pcm" | "mulaw" | "ulaw" | "alaw"
                                        ) {
                                            let bytes_per_sample = if matches!(format.as_str(), "linear16" | "pcm") { 2 } else { 1 };
                                            let samples = (bytes.len() / bytes_per_sample) as u32;
                                            Some((samples * 1000) / sample_rate)
                                        } else {
                                            None
                                        };

                                        let audio_data = AudioData {
                                            data: bytes,
                                            sample_rate,
                                            format: format.to_string(),
                                            duration_ms,
                                        };
                                        debug!("TTS dispatcher calling audio callback with {} bytes", audio_data.data.len());
                                        cb.on_audio(audio_data).await;
                                        debug!("TTS dispatcher audio callback completed");
                                    }
                                }
                                Err(err) => {
                                    error!("TTS dispatcher received error: {:?}", err);
                                    if let Some(cb) = cb_opt.as_ref() {
                                        cb.on_error(err).await;
                                    }
                                    break;
                                }
                            }
                        }
                        debug!("TTS dispatcher finished processing request with {} chunks", chunk_count);

                        // Only notify completion if this is the last request in the queue
                        // This ensures that multiple consecutive speak() calls only trigger
                        // one completion event after all audio has been generated
                        let is_last_request = {
                            let queue = pending_queue.lock().await;
                            queue.is_empty()
                        };

                        if is_last_request {
                            if let Some(cb) = cb_opt.as_ref() {
                                debug!("TTS dispatcher notifying completion (last request in queue)");
                                cb.on_complete().await;
                            }
                        } else {
                            debug!("TTS dispatcher skipping completion (more requests in queue)");
                        }
                    } => {}
                }
            }
        });

        *task_guard = Some(handle);
    }

    /// Spawn the sequential worker if not already running.
    ///
    /// The worker is responsible for processing speak jobs one at a time in FIFO order.
    /// Each job runs to completion (all audio chunks streamed) before the next job starts.
    async fn ensure_worker(&self) {
        let mut task_guard = self.worker_task.lock().await;
        if task_guard.is_some() {
            return;
        }

        let job_queue = self.job_queue.clone();
        let pending_queue = self.pending_queue.clone();
        let pending_notify = self.pending_notify.clone();
        let job_notify = self.job_notify.clone();
        let token = self.cancel_token.clone();

        let handle = tokio::spawn(async move {
            debug!("TTS sequential worker started");

            loop {
                tokio::select! {
                    _ = token.cancelled() => {
                        debug!("TTS worker cancelled");
                        break;
                    }
                    _ = async {
                        // Get next work item from job queue
                        let work_item = {
                            let mut queue = job_queue.lock().await;
                            queue.pop_front()
                        };

                        let Some(work_item) = work_item else {
                            // No pending jobs, wait for notification
                            job_notify.notified().await;
                            return;
                        };

                        debug!("TTS worker processing job from queue");

                        // Add the request entry to pending_queue FIRST
                        // This ensures the dispatcher can start processing as soon as data arrives
                        {
                            let mut queue = pending_queue.lock().await;
                            queue.push_back(work_item.entry);
                            debug!("TTS worker added entry to pending_queue, size: {}", queue.len());
                        }
                        // Notify dispatcher that a new request is available
                        pending_notify.notify_one();

                        // Execute the task (this will call send_request and stream all chunks)
                        // The task completes when all audio has been sent through the channel
                        debug!("TTS worker executing send_request task");
                        work_item.task.await;
                        debug!("TTS worker completed send_request task");

                        // Task complete, loop will move to next job
                    } => {}
                }
            }

            debug!("TTS sequential worker exited");
        });

        *task_guard = Some(handle);
    }

    /// Generic connect implementation with TTSConfig-based timeouts
    pub async fn generic_connect(&mut self, _default_url: &str) -> TTSResult<()> {
        // This method now expects the request manager to be pre-configured
        // with the correct timeouts and pool size from TTSConfig
        if self.req_manager.read().await.is_none() {
            return Err(TTSError::ConnectionFailed(
                "Request manager not configured. Call generic_connect_with_config instead."
                    .to_string(),
            ));
        }

        self.connected.store(true, Ordering::Relaxed);
        info!("TTS provider connected and ready");

        Ok(())
    }

    /// Connect with configuration-based request manager
    pub async fn generic_connect_with_config(
        &mut self,
        default_url: &str,
        config: &TTSConfig,
    ) -> TTSResult<()> {
        // Check if we have a request manager
        if self.req_manager.read().await.is_none() {
            // Create request manager with config-based settings
            let pool_size = config.request_pool_size.unwrap_or(4);
            let mut req_config = ReqManagerConfig {
                max_concurrent_requests: pool_size,
                ..Default::default()
            };

            // Apply timeout configurations
            if let Some(connect_timeout) = config.connection_timeout {
                req_config.connect_timeout = Duration::from_secs(connect_timeout);
            }
            if let Some(request_timeout) = config.request_timeout {
                req_config.request_timeout = Duration::from_secs(request_timeout);
            }

            match ReqManager::with_config(req_config).await {
                Ok(manager) => {
                    // Warm up connections
                    let _ = manager.warmup(default_url, "OPTIONS").await;
                    *self.req_manager.write().await = Some(Arc::new(manager));
                    info!(
                        "Created ReqManager with pool_size={}, connect_timeout={:?}s, request_timeout={:?}s",
                        pool_size, config.connection_timeout, config.request_timeout
                    );
                }
                Err(e) => {
                    return Err(TTSError::ConnectionFailed(format!(
                        "Failed to create request manager: {e}"
                    )));
                }
            }
        }

        self.connected.store(true, Ordering::Relaxed);
        info!("TTS provider connected and ready");

        Ok(())
    }

    /// Generic disconnect implementation
    pub async fn generic_disconnect(&mut self) -> TTSResult<()> {
        // Cancel worker
        {
            let mut worker = self.worker_task.lock().await;
            if let Some(handle) = worker.take() {
                handle.abort();
                let _ = handle.await;
            }
        }

        // Cancel dispatcher
        {
            let mut disp = self.dispatcher_task.lock().await;
            if let Some(handle) = disp.take() {
                handle.abort();
                let _ = handle.await;
            }
        }

        // Clear queues
        self.job_queue.lock().await.clear();
        self.pending_queue.lock().await.clear();

        // Clear state
        self.connected.store(false, Ordering::Relaxed);
        *self.audio_callback.write().await = None;
        *self.previous_text.write().await = None;

        info!("TTS provider disconnected");
        Ok(())
    }

    /// Generic speak implementation with request builder
    pub async fn generic_speak<R: TTSRequestBuilder + Clone + 'static>(
        &mut self,
        request_builder: R,
        text: &str,
        _flush: bool,
    ) -> TTSResult<()> {
        if !self.is_ready() {
            return Err(TTSError::ProviderNotReady(
                "Provider not connected".to_string(),
            ));
        }

        // Skip empty text
        if text.is_empty() || text.trim().is_empty() {
            return Ok(());
        }

        // Prepare text and generate hash
        let text_trimmed = text.trim().to_string();
        let text_hash = format!("{:032x}", xxh3_128(text_trimmed.as_bytes()));

        // Create channel for this request with buffer size of 1
        // This ensures sender waits for receiver to read each message (like Go's unbuffered channels)
        let (sender, receiver) = mpsc::channel::<Result<Vec<u8>, TTSError>>(1);

        // Get format and sample rate from current config
        let format = request_builder
            .get_config()
            .audio_format
            .clone()
            .unwrap_or_else(|| "linear16".to_string());
        let sample_rate = request_builder.get_config().sample_rate.unwrap_or(24000);

        // Create the request entry that will be added to pending_queue by the worker
        let entry = RequestEntry {
            receiver,
            format: format.clone(),
            sample_rate,
        };

        // Prepare cache key if cache is available
        let cache_opt = self.cache.read().await.clone();
        let config_hash_opt = self.tts_config_hash.read().await.clone();
        let cache_and_key = match (cache_opt, config_hash_opt) {
            (Some(cache), Some(cfg_hash)) => Some((cache, format!("{cfg_hash}:{text_hash}"))),
            _ => None,
        };

        // Get request manager
        let req_mgr_opt = self.req_manager.read().await.clone();
        let Some(req_mgr) = req_mgr_opt else {
            return Err(TTSError::InternalError(
                "Request manager not configured".to_string(),
            ));
        };

        let token = self.cancel_token.clone();
        let request_builder_clone = request_builder;
        let previous_text_store = self.previous_text.clone();

        // Only clone text for debug logging if needed
        let text_for_debug = if tracing::enabled!(tracing::Level::DEBUG) {
            Some(text_trimmed.clone())
        } else {
            None
        };

        // Create the task (future) that will execute send_request
        // This future will be executed by the worker
        let task = Box::pin(async move {
            Self::send_request(
                &request_builder_clone,
                req_mgr,
                text_trimmed,
                sender,
                token,
                cache_and_key,
                previous_text_store,
            )
            .await;
        });

        // Create work item
        let work_item = WorkItem { task, entry };

        // Add work item to job queue with backpressure
        let queue_size = {
            let mut queue = self.job_queue.lock().await;

            queue.push_back(work_item);
            let size = queue.len();

            if let Some(ref text) = text_for_debug {
                debug!(
                    "Added job to queue for text: '{}', queue size now: {}",
                    text, size
                );
            }

            size
        };

        // Notify worker that a new job is available (zero latency wake-up)
        self.job_notify.notify_one();

        // Warn if queue is growing (potential performance issue)
        if queue_size > 10 {
            warn!("TTS job queue growing large: {} items", queue_size);
        }

        // Ensure both worker and dispatcher are running
        // Worker must start first to process jobs and add entries to pending_queue
        // Dispatcher reads from pending_queue
        self.ensure_worker().await;
        self.ensure_dispatcher().await;

        Ok(())
    }

    /// Generic clear implementation
    pub async fn generic_clear(&mut self) -> TTSResult<()> {
        // Stop worker first (processes jobs)
        {
            let mut worker = self.worker_task.lock().await;
            if let Some(handle) = worker.take() {
                handle.abort();
                let _ = handle.await;
            }
        }

        // Stop dispatcher (delivers audio)
        {
            let mut disp = self.dispatcher_task.lock().await;
            if let Some(handle) = disp.take() {
                handle.abort();
                let _ = handle.await;
            }
        }

        // Cancel current generation (will stop any in-flight HTTP requests)
        self.cancel_token.cancel();

        // Clear both queues
        self.job_queue.lock().await.clear();
        self.pending_queue.lock().await.clear();

        // Clear previous_text context
        *self.previous_text.write().await = None;

        // Reset token for next cycle
        self.cancel_token = CancellationToken::new();

        debug!("Cleared job queue, pending queue, previous_text, cancelled worker and dispatcher");
        Ok(())
    }

    /// Check if ready
    pub fn is_ready(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    /// Get connection state
    pub fn get_connection_state(&self) -> ConnectionState {
        if self.connected.load(Ordering::Relaxed) {
            ConnectionState::Connected
        } else {
            ConnectionState::Disconnected
        }
    }

    /// Generic flush implementation
    pub async fn generic_flush(&self) -> TTSResult<()> {
        // No-op in concurrent mode; dispatcher processes items as they arrive
        Ok(())
    }

    /// Generic on_audio implementation
    pub fn generic_on_audio(&mut self, callback: Arc<dyn AudioCallback>) -> TTSResult<()> {
        // Use try_write to avoid blocking
        if let Ok(mut audio_callback) = self.audio_callback.try_write() {
            *audio_callback = Some(callback);
            Ok(())
        } else {
            Err(TTSError::InternalError(
                "Failed to register audio callback".to_string(),
            ))
        }
    }

    /// Generic remove_audio_callback implementation
    pub fn generic_remove_audio_callback(&mut self) -> TTSResult<()> {
        // Use try_write to avoid blocking
        if let Ok(mut audio_callback) = self.audio_callback.try_write() {
            *audio_callback = None;
            Ok(())
        } else {
            Err(TTSError::InternalError(
                "Failed to remove audio callback".to_string(),
            ))
        }
    }

    /// Set cache store for provider
    pub async fn set_cache(&mut self, cache: Arc<CacheStore>) {
        *self.cache.write().await = Some(cache);
    }

    /// Set precomputed TTS config hash for cache keying
    pub async fn set_tts_config_hash(&mut self, hash: String) {
        *self.tts_config_hash.write().await = Some(hash);
    }
}

impl Drop for TTSProvider {
    fn drop(&mut self) {
        // Best-effort cancel worker and dispatcher without awaiting
        if let Some(handle) = self
            .worker_task
            .try_lock()
            .ok()
            .and_then(|mut worker| worker.take())
        {
            handle.abort();
        }
        if let Some(handle) = self
            .dispatcher_task
            .try_lock()
            .ok()
            .and_then(|mut disp| disp.take())
        {
            handle.abort();
        }
        // Best-effort clear previous_text
        if let Ok(mut prev) = self.previous_text.try_write() {
            *prev = None;
        }
    }
}
