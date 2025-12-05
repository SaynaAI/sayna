use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use google_api_proto::google::cloud::speech::v2::speech_client::SpeechClient;
use tokio::sync::{Notify, RwLock, mpsc, oneshot};
use tracing::{debug, error, info};

use crate::core::providers::google::{
    CredentialSource, GOOGLE_CLOUD_PLATFORM_SCOPE, GOOGLE_SPEECH_ENDPOINT, GoogleAuthClient,
    GoogleError, TokenProvider, create_authenticated_channel,
};
use crate::core::stt::base::{
    BaseSTT, STTConfig, STTError, STTErrorCallback, STTResult, STTResultCallback,
};

use super::config::GoogleSTTConfig;
use super::streaming::{
    KEEPALIVE_INTERVAL_SECS, KeepaliveTracker, build_audio_request, build_config_request,
    chunk_audio, handle_grpc_error, handle_streaming_response,
};

/// Converts a GoogleError to an STTError.
pub(crate) fn google_error_to_stt(e: GoogleError) -> STTError {
    match e {
        GoogleError::AuthenticationFailed(msg) => STTError::AuthenticationFailed(msg),
        GoogleError::ConfigurationError(msg) => STTError::ConfigurationError(msg),
        GoogleError::ConnectionFailed(msg) => STTError::ConnectionFailed(msg),
        GoogleError::NetworkError(msg) => STTError::NetworkError(msg),
        GoogleError::ApiError(msg) => STTError::ProviderError(msg),
        GoogleError::GrpcError { code, message } => {
            STTError::ProviderError(format!("gRPC error ({code}): {message}"))
        }
    }
}

/// STT-specific wrapper for GoogleAuthClient.
///
/// Validates credentials and creates an auth client with the Speech API scope.
#[derive(Debug)]
pub struct STTGoogleAuthClient {
    inner: GoogleAuthClient,
}

impl STTGoogleAuthClient {
    /// Creates a new STT auth client from a credential source.
    pub fn new(credential_source: CredentialSource) -> Result<Self, STTError> {
        credential_source.validate().map_err(google_error_to_stt)?;

        let inner = GoogleAuthClient::new(credential_source, &[GOOGLE_CLOUD_PLATFORM_SCOPE])
            .map_err(google_error_to_stt)?;

        Ok(Self { inner })
    }

    /// Creates a new auth client from an API key string.
    pub fn from_api_key(api_key: &str) -> Result<Self, STTError> {
        let source = CredentialSource::from_api_key(api_key);
        Self::new(source)
    }
}

#[async_trait::async_trait]
impl TokenProvider for STTGoogleAuthClient {
    async fn get_token(&self) -> Result<String, GoogleError> {
        self.inner.get_token().await
    }
}

type AsyncSTTCallback = Box<
    dyn Fn(STTResult) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        + Send
        + Sync,
>;

type AsyncErrorCallback = Box<
    dyn Fn(STTError) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        + Send
        + Sync,
>;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    /// Error state - variant is constructed but not pattern-matched
    Error,
}

/// Audio channel buffer size - matches Deepgram's buffer for consistent behavior.
/// 32 provides good balance between latency and burst handling.
const AUDIO_CHANNEL_BUFFER_SIZE: usize = 32;

pub struct GoogleSTT {
    pub(super) config: Option<GoogleSTTConfig>,
    pub(super) state: ConnectionState,
    pub(super) state_notify: Arc<Notify>,
    /// Audio sender uses Bytes for zero-copy transfer
    pub(super) audio_sender: Option<mpsc::Sender<Bytes>>,
    pub(super) shutdown_tx: Option<oneshot::Sender<()>>,
    pub(super) result_tx: Option<mpsc::UnboundedSender<STTResult>>,
    /// Channel for propagating streaming errors to the client
    pub(super) error_tx: Option<mpsc::UnboundedSender<STTError>>,
    pub(super) connection_handle: Option<tokio::task::JoinHandle<()>>,
    pub(super) result_forward_handle: Option<tokio::task::JoinHandle<()>>,
    /// Handle for error forwarding task
    pub(super) error_forward_handle: Option<tokio::task::JoinHandle<()>>,
    /// Using RwLock instead of Mutex to reduce contention - callbacks are read-heavy
    pub(super) result_callback: Arc<RwLock<Option<AsyncSTTCallback>>>,
    /// Error callback for streaming errors
    pub(super) error_callback: Arc<RwLock<Option<AsyncErrorCallback>>>,
    pub(super) auth_client: Option<Arc<dyn TokenProvider>>,
}

impl Default for GoogleSTT {
    fn default() -> Self {
        Self {
            config: None,
            state: ConnectionState::Disconnected,
            state_notify: Arc::new(Notify::new()),
            audio_sender: None,
            shutdown_tx: None,
            result_tx: None,
            error_tx: None,
            connection_handle: None,
            result_forward_handle: None,
            error_forward_handle: None,
            result_callback: Arc::new(RwLock::new(None)),
            error_callback: Arc::new(RwLock::new(None)),
            auth_client: None,
        }
    }
}

impl GoogleSTT {
    pub(super) fn create_google_config(config: STTConfig, project_id: String) -> GoogleSTTConfig {
        GoogleSTTConfig {
            base: config,
            project_id,
            location: "global".to_string(),
            recognizer_id: None,
            interim_results: true,
            enable_voice_activity_events: true,
            speech_start_timeout: None,
            speech_end_timeout: None,
            single_utterance: false,
        }
    }

    async fn start_connection(&mut self, config: GoogleSTTConfig) -> Result<(), STTError> {
        let auth_client = self.auth_client.clone().ok_or_else(|| {
            STTError::AuthenticationFailed("Auth client not initialized".to_string())
        })?;

        // Use smaller buffer for lower latency - Bytes enables zero-copy
        let (audio_tx, audio_rx) = mpsc::channel::<Bytes>(AUDIO_CHANNEL_BUFFER_SIZE);
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let (result_tx, mut result_rx) = mpsc::unbounded_channel::<STTResult>();
        let (error_tx, mut error_rx) = mpsc::unbounded_channel::<STTError>();
        // Connection result channel - sends () on success when gRPC channel is established
        let (connected_tx, connected_rx) = oneshot::channel::<()>();

        self.audio_sender = Some(audio_tx);
        self.shutdown_tx = Some(shutdown_tx);
        self.result_tx = Some(result_tx.clone());
        self.error_tx = Some(error_tx.clone());

        // Pre-compute recognizer path once - avoid repeated allocations in hot path
        let recognizer_path = config.recognizer_path();
        let initial_config = build_config_request(&config);

        // Get sample rate and channels for keep-alive audio generation
        let sample_rate = config.base.sample_rate;
        let channels = config.base.channels as u32;

        let connection_handle = tokio::spawn(async move {
            // Use generic authenticated channel from providers
            let authenticated_channel =
                match create_authenticated_channel(GOOGLE_SPEECH_ENDPOINT, auth_client.clone())
                    .await
                {
                    Ok(client) => client,
                    Err(e) => {
                        let stt_error = google_error_to_stt(e);
                        error!("Failed to create speech client: {}", stt_error);
                        let _ = error_tx.send(stt_error);
                        return;
                    }
                };

            // Get initial auth header
            let auth_header = match authenticated_channel.get_authorization_header().await {
                Ok(header) => header,
                Err(e) => {
                    let stt_error = google_error_to_stt(e);
                    error!("Failed to get authorization header: {}", stt_error);
                    let _ = error_tx.send(stt_error);
                    return;
                }
            };

            // Pre-parse the header value once to avoid repeated parsing
            let auth_metadata_value: tonic::metadata::MetadataValue<_> = match auth_header.parse() {
                Ok(v) => v,
                Err(_) => {
                    let stt_error = STTError::AuthenticationFailed(
                        "Failed to parse authorization header".to_string(),
                    );
                    error!("{}", stt_error);
                    let _ = error_tx.send(stt_error);
                    return;
                }
            };

            let channel = authenticated_channel.clone_channel();
            let mut client =
                SpeechClient::with_interceptor(channel, move |mut req: tonic::Request<()>| {
                    req.metadata_mut()
                        .insert("authorization", auth_metadata_value.clone());
                    Ok(req)
                });

            info!(
                "Connected to Google Speech-to-Text API, recognizer: {}",
                recognizer_path
            );

            // Signal successful connection IMMEDIATELY after gRPC client is created
            // (like Deepgram does after WebSocket handshake completes)
            // This allows the main thread to proceed while streaming runs in background
            let _ = connected_tx.send(());

            // Clone recognizer path once for the stream
            let recognizer_for_stream = recognizer_path.clone();

            let request_stream = async_stream::stream! {
                debug!("Sending initial streaming configuration");
                yield initial_config;

                let mut shutdown_rx = shutdown_rx;
                let mut audio_rx = audio_rx;

                // Keep-alive mechanism to prevent Google's server-side timeout (~10 seconds).
                // Similar to Deepgram's keep-alive, but we send silent audio frames instead of JSON.
                let mut keepalive_timer = tokio::time::interval(Duration::from_secs(KEEPALIVE_INTERVAL_SECS));
                let mut keepalive_tracker = KeepaliveTracker::new(sample_rate, channels);

                loop {
                    tokio::select! {
                        // Prioritize audio data (biased) for lowest latency
                        biased;

                        audio_opt = audio_rx.recv() => {
                            match audio_opt {
                                Some(audio_data) => {
                                    // Update activity tracker
                                    keepalive_tracker.touch();
                                    // Use iterator to avoid Vec allocation
                                    for chunk in chunk_audio(audio_data) {
                                        debug!("Sending {} bytes of audio data", chunk.len());
                                        yield build_audio_request(chunk, recognizer_for_stream.clone());
                                    }
                                }
                                None => {
                                    debug!("Audio channel closed, ending request stream");
                                    break;
                                }
                            }
                        }

                        // Handle keep-alive timer - send silent audio to prevent timeout
                        _ = keepalive_timer.tick() => {
                            if keepalive_tracker.needs_keepalive() {
                                let silence = keepalive_tracker.generate_keepalive();
                                debug!("Sending keep-alive silence ({} bytes) to Google STT", silence.len());
                                yield build_audio_request(silence, recognizer_for_stream.clone());
                                keepalive_tracker.touch();
                            }
                        }

                        _ = &mut shutdown_rx => {
                            info!("Shutdown signal received, ending request stream");
                            break;
                        }
                    }
                }
            };

            let response = match client.streaming_recognize(request_stream).await {
                Ok(r) => r,
                Err(e) => {
                    let stt_error = handle_grpc_error(e);
                    error!("Failed to start streaming recognition: {}", stt_error);
                    let _ = error_tx.send(stt_error);
                    return;
                }
            };

            let mut response_stream = response.into_inner();

            // Process all incoming messages from Google STT
            // Permission/auth errors will come through as gRPC errors and be sent via error_tx
            loop {
                match response_stream.message().await {
                    Ok(Some(msg)) => {
                        if let Err(e) = handle_streaming_response(msg, &result_tx) {
                            error!("Error handling streaming response: {}", e);
                            let _ = error_tx.send(e);
                            break;
                        }
                    }
                    Ok(None) => {
                        info!("Google Speech-to-Text stream ended");
                        break;
                    }
                    Err(status) => {
                        let stt_error = handle_grpc_error(status);
                        error!("Streaming error from Google STT: {}", stt_error);
                        let _ = error_tx.send(stt_error);
                        break;
                    }
                }
            }

            info!("Google Speech-to-Text connection closed");
        });

        self.connection_handle = Some(connection_handle);

        // Callback forwarding task - acquires lock once per result
        let callback_ref = self.result_callback.clone();
        let result_forward_handle = tokio::spawn(async move {
            while let Some(result) = result_rx.recv().await {
                // Single lock acquisition - clone the callback if present to release lock before await
                let callback_opt = {
                    let guard = callback_ref.read().await;
                    guard.as_ref().map(|cb| {
                        // Create the future while holding the lock, but don't await it
                        cb(result.clone())
                    })
                };

                if let Some(future) = callback_opt {
                    // Execute the callback future without holding any lock
                    future.await;
                } else {
                    debug!(
                        "Received STT result but no callback registered: {} (confidence: {})",
                        result.transcript, result.confidence
                    );
                }
            }
        });

        self.result_forward_handle = Some(result_forward_handle);

        // Error forwarding task - propagates streaming errors to registered callback
        let error_callback_ref = self.error_callback.clone();
        let error_forward_handle = tokio::spawn(async move {
            while let Some(error) = error_rx.recv().await {
                // Single lock acquisition - clone the callback if present to release lock before await
                let callback_opt = {
                    let guard = error_callback_ref.read().await;
                    guard.as_ref().map(|cb| cb(error.clone()))
                };

                if let Some(future) = callback_opt {
                    // Execute the callback future without holding any lock
                    future.await;
                } else {
                    error!(
                        "STT streaming error but no error callback registered: {}",
                        error
                    );
                }
            }
        });

        self.error_forward_handle = Some(error_forward_handle);

        self.state = ConnectionState::Connecting;

        // Wait for gRPC channel to be established (like Deepgram waits for WebSocket handshake)
        // Any permission/auth errors will come through the error_tx channel asynchronously
        match tokio::time::timeout(Duration::from_secs(30), connected_rx).await {
            Ok(Ok(())) => {
                // gRPC channel established - stream is ready to receive audio
                self.state = ConnectionState::Connected;
                self.state_notify.notify_waiters();
                info!("Successfully connected to Google Speech-to-Text");
                Ok(())
            }
            Ok(Err(_)) => {
                let error_msg = "Connection channel closed unexpectedly".to_string();
                self.state = ConnectionState::Error;
                Err(STTError::ConnectionFailed(error_msg))
            }
            Err(_) => {
                let error_msg = "Connection timeout (30s)".to_string();
                self.state = ConnectionState::Error;
                Err(STTError::ConnectionFailed(error_msg))
            }
        }
    }
}

#[async_trait::async_trait]
impl BaseSTT for GoogleSTT {
    fn new(config: STTConfig) -> Result<Self, STTError>
    where
        Self: Sized,
    {
        // First, determine the credential source so we can extract project_id from it
        let credential_source = CredentialSource::from_api_key(&config.api_key);

        let (project_id, model_name) = if config.model.contains(':') {
            // Project ID explicitly provided in model field: "project_id:model_name"
            let parts: Vec<&str> = config.model.splitn(2, ':').collect();
            (parts[0].to_string(), parts[1].to_string())
        } else {
            // Try to extract project_id from credentials (service account JSON has project_id field)
            let project_id = credential_source.extract_project_id().unwrap_or_default();
            (project_id, config.model.clone())
        };

        if project_id.is_empty() {
            return Err(STTError::ConfigurationError(
                "Google Cloud project_id is required. Provide it either:\n\
                 1. In the model field as 'project_id:model_name'\n\
                 2. In the service account credentials JSON (project_id field)"
                    .to_string(),
            ));
        }

        let auth_client = STTGoogleAuthClient::new(credential_source)?;

        let mut updated_config = config;
        updated_config.model = model_name;
        let google_config = Self::create_google_config(updated_config, project_id);

        Ok(Self {
            config: Some(google_config),
            state: ConnectionState::Disconnected,
            state_notify: Arc::new(Notify::new()),
            audio_sender: None,
            shutdown_tx: None,
            result_tx: None,
            error_tx: None,
            connection_handle: None,
            result_forward_handle: None,
            error_forward_handle: None,
            result_callback: Arc::new(RwLock::new(None)),
            error_callback: Arc::new(RwLock::new(None)),
            auth_client: Some(Arc::new(auth_client)),
        })
    }

    async fn connect(&mut self) -> Result<(), STTError> {
        let config = self.config.as_ref().ok_or_else(|| {
            STTError::ConfigurationError("No configuration available".to_string())
        })?;

        self.start_connection(config.clone()).await
    }

    async fn disconnect(&mut self) -> Result<(), STTError> {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }

        if let Some(handle) = self.connection_handle.take() {
            let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
        }

        if let Some(handle) = self.result_forward_handle.take() {
            handle.abort();
            let _ = handle.await;
        }

        if let Some(handle) = self.error_forward_handle.take() {
            handle.abort();
            let _ = handle.await;
        }

        self.audio_sender = None;
        self.result_tx = None;
        self.error_tx = None;
        *self.result_callback.write().await = None;
        *self.error_callback.write().await = None;

        self.state = ConnectionState::Disconnected;
        self.state_notify.notify_waiters();

        info!("Disconnected from Google Speech-to-Text");
        Ok(())
    }

    fn is_ready(&self) -> bool {
        matches!(self.state, ConnectionState::Connected) && self.audio_sender.is_some()
    }

    async fn send_audio(&mut self, audio_data: Vec<u8>) -> Result<(), STTError> {
        if !self.is_ready() {
            return Err(STTError::ConnectionFailed(
                "Not connected to Google Speech-to-Text".to_string(),
            ));
        }

        if let Some(audio_sender) = &self.audio_sender {
            let data_len = audio_data.len();

            // Convert to Bytes for zero-copy transfer through the channel
            let audio_bytes = Bytes::from(audio_data);

            // Send audio data with backpressure handling (matches Deepgram pattern)
            audio_sender
                .send(audio_bytes)
                .await
                .map_err(|e| STTError::NetworkError(format!("Failed to send audio data: {e}")))?;

            debug!("Sent {} bytes of audio data to Google STT", data_len);
        }

        Ok(())
    }

    async fn on_result(&mut self, callback: STTResultCallback) -> Result<(), STTError> {
        // Use write lock since we're modifying the callback
        *self.result_callback.write().await = Some(Box::new(move |result| {
            let cb = callback.clone();
            Box::pin(async move {
                cb(result).await;
            })
        }));
        Ok(())
    }

    async fn on_error(&mut self, callback: STTErrorCallback) -> Result<(), STTError> {
        // Use write lock since we're modifying the callback
        *self.error_callback.write().await = Some(Box::new(move |error| {
            let cb = callback.clone();
            Box::pin(async move {
                cb(error).await;
            })
        }));
        Ok(())
    }

    fn get_config(&self) -> Option<&STTConfig> {
        self.config.as_ref().map(|c| &c.base)
    }

    async fn update_config(&mut self, config: STTConfig) -> Result<(), STTError> {
        if self.is_ready() {
            self.disconnect().await?;
        }

        // First, determine the credential source so we can extract project_id from it
        let credential_source = CredentialSource::from_api_key(&config.api_key);

        let (project_id, model_name) = if config.model.contains(':') {
            // Project ID explicitly provided in model field: "project_id:model_name"
            let parts: Vec<&str> = config.model.splitn(2, ':').collect();
            (parts[0].to_string(), parts[1].to_string())
        } else {
            // Try to extract project_id from credentials (service account JSON has project_id field)
            let project_id = credential_source.extract_project_id().unwrap_or_default();
            (project_id, config.model.clone())
        };

        if project_id.is_empty() {
            return Err(STTError::ConfigurationError(
                "Google Cloud project_id is required. Provide it either:\n\
                 1. In the model field as 'project_id:model_name'\n\
                 2. In the service account credentials JSON (project_id field)"
                    .to_string(),
            ));
        }

        let auth_client = STTGoogleAuthClient::new(credential_source)?;
        self.auth_client = Some(Arc::new(auth_client));

        let mut updated_config = config;
        updated_config.model = model_name;
        self.config = Some(Self::create_google_config(updated_config, project_id));

        self.connect().await?;
        Ok(())
    }

    fn get_provider_info(&self) -> &'static str {
        "Google Cloud Speech-to-Text v2"
    }
}

impl Drop for GoogleSTT {
    fn drop(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
    }
}
