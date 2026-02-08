use axum::{
    extract::State,
    http::{HeaderName, StatusCode, header},
    response::{IntoResponse, Json, Response},
};
use serde::Deserialize;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info};

use crate::core::tts::{AudioCallback, AudioData, TTSError, create_tts_provider};
use crate::handlers::ws::config::TTSWebSocketConfig;
use crate::state::AppState;

/// Request body for the speak endpoint
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SpeakRequest {
    /// The text to synthesize
    #[cfg_attr(feature = "openapi", schema(example = "Hello, world!"))]
    pub text: String,
    /// TTS configuration (without API key)
    pub tts_config: TTSWebSocketConfig,
}

/// Collector for accumulating audio from TTS provider
struct AudioCollector {
    audio_data: Arc<Mutex<Vec<u8>>>,
    format: Arc<Mutex<Option<String>>>,
    sample_rate: Arc<Mutex<Option<u32>>>,
    completed: Arc<Mutex<bool>>,
    error: Arc<Mutex<Option<TTSError>>>,
}

impl AudioCollector {
    fn new() -> Self {
        Self {
            audio_data: Arc::new(Mutex::new(Vec::new())),
            format: Arc::new(Mutex::new(None)),
            sample_rate: Arc::new(Mutex::new(None)),
            completed: Arc::new(Mutex::new(false)),
            error: Arc::new(Mutex::new(None)),
        }
    }

    async fn wait_for_completion(&self) {
        while !*self.completed.lock().await {
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }
    }

    async fn get_result(&self) -> Result<(Vec<u8>, String, u32), TTSError> {
        if let Some(err) = self.error.lock().await.clone() {
            return Err(err);
        }

        let audio = self.audio_data.lock().await.clone();
        let format = self
            .format
            .lock()
            .await
            .clone()
            .unwrap_or_else(|| "linear16".to_string());
        let sample_rate = self.sample_rate.lock().await.unwrap_or(24000);

        Ok((audio, format, sample_rate))
    }
}

impl AudioCallback for AudioCollector {
    fn on_audio(
        &self,
        audio_data: AudioData,
    ) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            // Store format and sample rate from first chunk
            if self.format.lock().await.is_none() {
                *self.format.lock().await = Some(audio_data.format.clone());
                *self.sample_rate.lock().await = Some(audio_data.sample_rate);
            }

            // Accumulate audio data
            self.audio_data
                .lock()
                .await
                .extend_from_slice(&audio_data.data);
        })
    }

    fn on_error(
        &self,
        error: TTSError,
    ) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            *self.error.lock().await = Some(error);
            *self.completed.lock().await = true;
        })
    }

    fn on_complete(&self) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            *self.completed.lock().await = true;
        })
    }
}

/// Handler for the /speak endpoint
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/speak",
        request_body = SpeakRequest,
        responses(
            (status = 200, description = "Audio generated successfully",
                content_type = "audio/pcm",
                headers(
                    ("x-audio-format" = String, description = "Audio format (linear16, mp3, etc.)"),
                    ("x-sample-rate" = u32, description = "Sample rate in Hz")
                )
            ),
            (status = 400, description = "Invalid request (empty text)"),
            (status = 500, description = "TTS synthesis failed")
        ),
        security(
            ("auth" = [])
        ),
        tag = "tts"
    )
)]
pub async fn speak_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SpeakRequest>,
) -> Response {
    info!(
        "Speak request received - provider: {}, text length: {}",
        request.tts_config.provider,
        request.text.len()
    );

    // Validate text
    if request.text.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Text cannot be empty"
            })),
        )
            .into_response();
    }

    // Get API key from config
    let api_key = match state.config.get_api_key(&request.tts_config.provider) {
        Ok(key) => key,
        Err(e) => {
            error!(
                "Failed to get API key for {}: {}",
                request.tts_config.provider, e
            );
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("API key not configured for provider: {}", request.tts_config.provider)
                })),
            )
                .into_response();
        }
    };

    // Convert WebSocket config to full TTSConfig with API key
    let tts_config = request.tts_config.to_tts_config(api_key);

    // Apply pronunciation replacements
    let mut processed_text = request.text.clone();
    for pronunciation in &tts_config.pronunciations {
        processed_text = processed_text.replace(&pronunciation.word, &pronunciation.pronunciation);
    }

    // Create TTS provider
    let mut tts_provider = match create_tts_provider(&tts_config.provider, tts_config.clone()) {
        Ok(provider) => provider,
        Err(e) => {
            error!("Failed to create TTS provider: {:?}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to create TTS provider: {}", e)
                })),
            )
                .into_response();
        }
    };

    // Set the request manager from state if available
    let req_manager = state.get_tts_req_manager(&tts_config.provider).await;
    if let (Some(provider), Some(req_manager)) = (tts_provider.get_provider(), req_manager) {
        provider.set_req_manager(req_manager).await;
    }

    // Connect to provider
    if let Err(e) = tts_provider.connect().await {
        error!("Failed to connect to TTS provider: {:?}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to connect to TTS provider: {}", e)
            })),
        )
            .into_response();
    }

    // Create audio collector
    let collector = Arc::new(AudioCollector::new());

    // Register callback
    if let Err(e) = tts_provider.on_audio(collector.clone()) {
        error!("Failed to register audio callback: {:?}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to register audio callback: {}", e)
            })),
        )
            .into_response();
    }

    // Synthesize speech
    if let Err(e) = tts_provider.speak(&processed_text, true).await {
        error!("Failed to synthesize speech: {:?}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to synthesize speech: {}", e)
            })),
        )
            .into_response();
    }

    // Wait for completion
    collector.wait_for_completion().await;

    // Disconnect
    let _ = tts_provider.disconnect().await;

    // Get result
    let (audio_data, format, sample_rate) = match collector.get_result().await {
        Ok(result) => result,
        Err(e) => {
            error!("TTS synthesis error: {:?}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("TTS synthesis error: {}", e)
                })),
            )
                .into_response();
        }
    };

    info!(
        "TTS synthesis successful - {} bytes, format: {}, sample_rate: {}",
        audio_data.len(),
        format,
        sample_rate
    );

    // Determine content type
    let content_type = match format.as_str() {
        "wav" => "audio/wav",
        "mp3" | "mpeg" => "audio/mpeg",
        "ogg" | "opus" => "audio/ogg",
        "linear16" | "pcm" => "audio/pcm",
        "mulaw" => "audio/basic",
        _ => "application/octet-stream",
    };

    // Return binary audio with headers
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            (
                header::CONTENT_LENGTH,
                audio_data.len().to_string().as_str(),
            ),
            (HeaderName::from_static("x-audio-format"), format.as_str()),
            (
                HeaderName::from_static("x-sample-rate"),
                sample_rate.to_string().as_str(),
            ),
        ],
        audio_data,
    )
        .into_response()
}
