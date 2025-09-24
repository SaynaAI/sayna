use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures_util::StreamExt;
use tokio::sync::{Mutex, RwLock, mpsc};
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};

use super::base::{AudioCallback, AudioData, ConnectionState, TTSConfig, TTSError, TTSResult};
use crate::core::cache::store::CacheStore;
use crate::utils::req_manager::ReqManager;
use xxhash_rust::xxh3::xxh3_128;

/// Request entry for ordered processing
struct RequestEntry {
    receiver: mpsc::Receiver<Result<Vec<u8>, TTSError>>,
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

    /// Get the configuration for this request builder
    fn get_config(&self) -> &TTSConfig;
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
    /// Per-text request tasks (managed as a join set)
    request_tasks: Arc<Mutex<JoinSet<()>>>,
    /// Dispatcher task that delivers audio in-order to the callback
    dispatcher_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// Cancellation token that controls current generation lifecycle
    cancel_token: CancellationToken,
    /// Optional cache store for TTS audio
    cache: Arc<RwLock<Option<Arc<CacheStore>>>>,
    /// Precomputed hash for the TTS configuration (stable for provider lifetime)
    tts_config_hash: Arc<RwLock<Option<String>>>,
}

impl TTSProvider {
    /// Create a new HTTP-based TTS provider instance
    pub fn new() -> TTSResult<Self> {
        Ok(Self {
            connected: Arc::new(AtomicBool::new(false)),
            audio_callback: Arc::new(RwLock::new(None)),
            pending_queue: Arc::new(Mutex::new(VecDeque::new())),
            req_manager: Arc::new(RwLock::new(None)),
            request_tasks: Arc::new(Mutex::new(JoinSet::new())),
            dispatcher_task: Arc::new(Mutex::new(None)),
            cancel_token: CancellationToken::new(),
            cache: Arc::new(RwLock::new(None)),
            tts_config_hash: Arc::new(RwLock::new(None)),
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
    ) {
        if token.is_cancelled() {
            let _ = sender
                .send(Err(TTSError::InternalError("Cancelled".to_string())))
                .await;
            return;
        }

        // Apply pronunciation replacements
        let config = request_builder.get_config();
        let mut processed_text = text.clone();
        for pronunciation in &config.pronunciations {
            processed_text =
                processed_text.replace(&pronunciation.word, &pronunciation.pronunciation);
        }

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
        let request = request_builder.build_http_request(client_guard.client(), &processed_text);

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
                let mut full_audio: Vec<u8> = Vec::new();

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
                                full_audio.extend_from_slice(&chunk_vec);
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
                                    full_audio.extend_from_slice(&chunk);
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
                    full_audio.extend_from_slice(&buffer);
                    debug!(
                        "Sending final buffer ({} bytes) - will wait for receiver...",
                        buffer.len()
                    );
                    let _ = sender.send(Ok(buffer)).await;
                    debug!("Final buffer sent and received");
                }

                // Store the full audio in cache if provided
                if let Some((cache, key)) = cache_and_key {
                    if let Err(e) = cache.put(key, full_audio).await {
                        error!("Failed to cache TTS audio: {:?}", e);
                    }
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
    async fn ensure_dispatcher<R: TTSRequestBuilder + Clone + 'static>(&self, request_builder: R) {
        let mut task_guard = self.dispatcher_task.lock().await;
        if task_guard.is_some() {
            return;
        }

        let pending_queue = self.pending_queue.clone();
        let audio_callback = self.audio_callback.clone();
        let config = request_builder.get_config().clone();
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
                            // No pending items, wait a bit
                            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                            return;
                        };

                        debug!("TTS dispatcher processing request from queue");

                        // Get format and sample rate for duration calculation
                        let (format, sample_rate) = (
                            config.audio_format.as_deref().unwrap_or("linear16"),
                            config.sample_rate.unwrap_or(24000),
                        );

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
                                            format,
                                            "linear16" | "pcm" | "mulaw" | "ulaw" | "alaw"
                                        ) {
                                            let bytes_per_sample = if matches!(format, "linear16" | "pcm") { 2 } else { 1 };
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

                        // Notify completion for this entry
                        if let Some(cb) = cb_opt.as_ref() {
                            debug!("TTS dispatcher notifying completion");
                            cb.on_complete().await;
                        }
                    } => {}
                }
            }
        });

        *task_guard = Some(handle);
    }

    /// Generic connect implementation
    pub async fn generic_connect(&mut self, default_url: &str) -> TTSResult<()> {
        // Check if we have a request manager
        if self.req_manager.read().await.is_none() {
            // If no external request manager provided, create a default one
            match ReqManager::new(10).await {
                Ok(manager) => {
                    // Warm up connections
                    let _ = manager.warmup(default_url, "OPTIONS").await;
                    *self.req_manager.write().await = Some(Arc::new(manager));
                    info!("Created default ReqManager for TTS provider");
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
        // Cancel dispatcher
        {
            let mut disp = self.dispatcher_task.lock().await;
            if let Some(handle) = disp.take() {
                handle.abort();
                let _ = handle.await;
            }
        }

        // Cancel all request tasks
        {
            let mut tasks = self.request_tasks.lock().await;
            tasks.abort_all();
        }

        // Clear queues
        self.pending_queue.lock().await.clear();

        // Clear state
        self.connected.store(false, Ordering::Relaxed);
        *self.audio_callback.write().await = None;

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

        // Add to pending queue FIRST before starting dispatcher
        let _queue_size = {
            let mut queue = self.pending_queue.lock().await;
            queue.push_back(RequestEntry { receiver });
            let size = queue.len();
            debug!(
                "Added request to pending queue for text: '{}', queue size now: {}",
                text_trimmed, size
            );
            size
        };

        // IMPORTANT: Ensure dispatcher is running BEFORE spawning request task
        // This is critical for cached audio which returns immediately
        self.ensure_dispatcher(request_builder.clone()).await;

        // Prepare cache key if cache is available
        let cache_opt = self.cache.read().await.clone();
        let config_hash_opt = self.tts_config_hash.read().await.clone();
        let cache_and_key = match (cache_opt, config_hash_opt) {
            (Some(cache), Some(cfg_hash)) => Some((cache, format!("{cfg_hash}:{text_hash}"))),
            _ => None,
        };

        // Spawn request task
        let req_mgr_opt = self.req_manager.read().await.clone();
        let Some(req_mgr) = req_mgr_opt else {
            return Err(TTSError::InternalError(
                "Request manager not configured".to_string(),
            ));
        };

        let token = self.cancel_token.clone();
        let request_builder_clone = request_builder;

        self.request_tasks.lock().await.spawn(async move {
            Self::send_request(
                &request_builder_clone,
                req_mgr,
                text_trimmed,
                sender,
                token,
                cache_and_key,
            )
            .await;
        });

        Ok(())
    }

    /// Generic clear implementation
    pub async fn generic_clear(&mut self) -> TTSResult<()> {
        // Stop dispatcher first
        {
            let mut disp = self.dispatcher_task.lock().await;
            if let Some(handle) = disp.take() {
                handle.abort();
                let _ = handle.await;
            }
        }

        // Cancel current generation and abort all inflight request tasks
        self.cancel_token.cancel();
        self.request_tasks.lock().await.abort_all();

        // Clear pending queue
        self.pending_queue.lock().await.clear();

        // Reset token for next cycle
        self.cancel_token = CancellationToken::new();

        debug!("Cleared pending items, cancelled request tasks, and stopped dispatcher");
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
        // Best-effort cancel dispatcher and tasks without awaiting
        if let Ok(mut disp) = self.dispatcher_task.try_lock() {
            if let Some(handle) = disp.take() {
                handle.abort();
            }
        }
        if let Ok(mut tasks) = self.request_tasks.try_lock() {
            tasks.abort_all();
        }
    }
}
