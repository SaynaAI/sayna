use axum::{extract::State, http::StatusCode, response::Json};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};

use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Voice {
    /// Voice ID or canonical name
    #[cfg_attr(feature = "openapi", schema(example = "aura-asteria-en"))]
    pub id: String,
    /// URL to sample audio
    #[cfg_attr(
        feature = "openapi",
        schema(example = "https://example.com/sample.mp3")
    )]
    pub sample: String,
    /// Display name of the voice
    #[cfg_attr(feature = "openapi", schema(example = "Asteria"))]
    pub name: String,
    /// Accent or dialect
    #[cfg_attr(feature = "openapi", schema(example = "American"))]
    pub accent: String,
    /// Gender of the voice
    #[cfg_attr(feature = "openapi", schema(example = "Female"))]
    pub gender: String,
    /// Language supported by the voice
    #[cfg_attr(feature = "openapi", schema(example = "English"))]
    pub language: String,
}

pub type VoicesResponse = HashMap<String, Vec<Voice>>;

// ElevenLabs API response structures
#[derive(Debug, Deserialize)]
struct ElevenLabsVoicesResponse {
    voices: Vec<ElevenLabsVoice>,
}

#[derive(Debug, Deserialize)]
struct ElevenLabsVoice {
    voice_id: String,
    name: String,
    preview_url: Option<String>,
    description: Option<String>,
    labels: Option<HashMap<String, String>>,
    verified_languages: Option<Vec<ElevenLabsLanguage>>,
}

#[derive(Debug, Deserialize)]
struct ElevenLabsLanguage {
    language: String,
    accent: Option<String>,
}

// Deepgram API response structures
#[derive(Debug, Deserialize)]
struct DeepgramModelsResponse {
    tts: Option<Vec<DeepgramTtsModel>>,
}

#[derive(Debug, Deserialize)]
struct DeepgramTtsModel {
    name: String,
    canonical_name: String,
    languages: Vec<String>,
    metadata: Option<DeepgramMetadata>,
}

#[derive(Debug, Deserialize)]
struct DeepgramMetadata {
    accent: Option<String>,
    sample: Option<String>,
    tags: Option<Vec<String>>,
}

// Helper function to fetch voices from ElevenLabs API
async fn fetch_elevenlabs_voices(
    api_key: &str,
) -> Result<Vec<Voice>, Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::Client::new();

    let response = client
        .get("https://api.elevenlabs.io/v2/voices")
        .header("xi-api-key", api_key)
        .send()
        .await?;

    let elevenlabs_response: ElevenLabsVoicesResponse = response.json().await?;

    let voices = elevenlabs_response
        .voices
        .into_iter()
        .map(|voice| {
            // Extract language and accent information from verified_languages
            let (language, accent) = if let Some(verified_languages) = &voice.verified_languages {
                if let Some(first_lang) = verified_languages.first() {
                    (
                        first_lang.language.clone(),
                        first_lang
                            .accent
                            .clone()
                            .unwrap_or_else(|| "Unknown".to_string()),
                    )
                } else {
                    ("Unknown".to_string(), "Unknown".to_string())
                }
            } else {
                ("Unknown".to_string(), "Unknown".to_string())
            };

            // Extract gender from labels or description
            let gender = voice
                .labels
                .as_ref()
                .and_then(|labels| {
                    // Check common gender keys in labels
                    for key in ["gender", "sex", "voice_type"] {
                        if let Some(value) = labels.get(key) {
                            let value_lower = value.to_lowercase();
                            if value_lower.contains("male") && !value_lower.contains("female") {
                                return Some("Male".to_string());
                            }
                            if value_lower.contains("female") && !value_lower.contains("male") {
                                return Some("Female".to_string());
                            }
                        }
                    }
                    None
                })
                .or_else(|| {
                    // Check description for gender keywords
                    voice.description.as_ref().and_then(|desc| {
                        let desc_lower = desc.to_lowercase();
                        if (desc_lower.contains("male") && !desc_lower.contains("female"))
                            || desc_lower.contains("masculine")
                            || desc_lower.contains(" man ")
                            || desc_lower.contains("gentleman")
                        {
                            Some("Male".to_string())
                        } else if (desc_lower.contains("female") && !desc_lower.contains("male"))
                            || desc_lower.contains("feminine")
                            || desc_lower.contains(" woman ")
                            || desc_lower.contains("lady")
                        {
                            Some("Female".to_string())
                        } else {
                            None
                        }
                    })
                })
                .unwrap_or_else(|| "Unknown".to_string());

            Voice {
                id: voice.voice_id,
                sample: voice.preview_url.unwrap_or_default(),
                name: voice.name,
                accent,
                gender,
                language,
            }
        })
        .collect();

    Ok(voices)
}

// Helper function to fetch voices from Deepgram API
async fn fetch_deepgram_voices(
    api_key: &str,
) -> Result<Vec<Voice>, Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::Client::new();

    let response = client
        .get("https://api.deepgram.com/v1/models")
        .header("Authorization", format!("Token {api_key}"))
        .send()
        .await?;

    let deepgram_response: DeepgramModelsResponse = response.json().await?;

    let voices = deepgram_response
        .tts
        .unwrap_or_default()
        .into_iter()
        .map(|model| {
            let metadata = model.metadata.as_ref();

            // Extract accent
            let accent = metadata
                .and_then(|m| m.accent.clone())
                .unwrap_or_else(|| "Unknown".to_string());

            // Extract sample URL
            let sample = metadata.and_then(|m| m.sample.clone()).unwrap_or_default();

            // Determine gender from tags
            let gender = metadata
                .and_then(|m| m.tags.as_ref())
                .and_then(|tags| {
                    for tag in tags {
                        let tag_lower = tag.to_lowercase();
                        if tag_lower.contains("masculine") || tag_lower.contains("male") {
                            return Some("Male".to_string());
                        }
                        if tag_lower.contains("feminine") || tag_lower.contains("female") {
                            return Some("Female".to_string());
                        }
                    }
                    None
                })
                .unwrap_or_else(|| "Unknown".to_string());

            // Extract language (use first available language)
            let language = model
                .languages
                .first()
                .map(|lang| {
                    // Convert language codes like "en" or "en-US" to readable format
                    if lang.starts_with("en") {
                        "English".to_string()
                    } else {
                        lang.clone()
                    }
                })
                .unwrap_or_else(|| "Unknown".to_string());

            Voice {
                id: model.canonical_name,
                sample,
                name: model.name,
                accent,
                gender,
                language,
            }
        })
        .collect();

    Ok(voices)
}

/// Handler for GET /voices - returns available voices per provider
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/voices",
        responses(
            (status = 200, description = "Available voices grouped by provider", body = HashMap<String, Vec<Voice>>),
            (status = 500, description = "Internal server error")
        ),
        security(
            ("bearer_auth" = [])
        ),
        tag = "voices"
    )
)]
pub async fn list_voices(
    State(state): State<Arc<AppState>>,
) -> Result<Json<VoicesResponse>, StatusCode> {
    let mut voices_response = HashMap::new();

    // Fetch ElevenLabs voices from API - fail if API key not configured or API call fails
    let api_key = state.config.get_api_key("elevenlabs").map_err(|e| {
        tracing::error!("ElevenLabs API key not configured: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let elevenlabs_voices = fetch_elevenlabs_voices(&api_key).await.map_err(|e| {
        tracing::error!("Failed to fetch ElevenLabs voices: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Fetch Deepgram voices from API - fail if API key not configured or API call fails
    let deepgram_api_key = state.config.get_api_key("deepgram").map_err(|e| {
        tracing::error!("Deepgram API key not configured: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let deepgram_voices = fetch_deepgram_voices(&deepgram_api_key)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch Deepgram voices: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    voices_response.insert("elevenlabs".to_string(), elevenlabs_voices);
    voices_response.insert("deepgram".to_string(), deepgram_voices);

    Ok(Json(voices_response))
}
