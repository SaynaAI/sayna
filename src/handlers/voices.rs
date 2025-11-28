use axum::{extract::State, http::StatusCode, response::Json};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};

use crate::core::providers::google::{
    CredentialSource, GOOGLE_CLOUD_PLATFORM_SCOPE, GoogleAuthClient, TokenProvider,
};
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

// Google TTS API response structures
#[derive(Debug, Deserialize)]
struct GoogleVoicesResponse {
    voices: Option<Vec<GoogleVoice>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleVoice {
    language_codes: Vec<String>,
    name: String,
    ssml_gender: Option<String>,
}

// Azure TTS Voices API response structures
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct AzureVoice {
    /// Full voice name, e.g., "Microsoft Server Speech Text to Speech Voice (en-US, JennyNeural)"
    #[allow(dead_code)]
    name: String,
    /// Display name, e.g., "Jenny"
    display_name: String,
    /// Short name used as voice ID, e.g., "en-US-JennyNeural"
    short_name: String,
    /// Gender: "Female" or "Male"
    gender: String,
    /// Locale code, e.g., "en-US"
    locale: String,
    /// Voice type, e.g., "Neural"
    #[allow(dead_code)]
    voice_type: String,
}

/// Maps a language code (e.g., "en-US") to a human-readable language name.
fn language_code_to_name(code: &str) -> String {
    // Extract the primary language code (e.g., "en" from "en-US")
    let primary = code.split('-').next().unwrap_or(code);

    match primary {
        "af" => "Afrikaans",
        "am" => "Amharic",
        "ar" => "Arabic",
        "bg" => "Bulgarian",
        "bn" => "Bengali",
        "ca" => "Catalan",
        "cmn" | "zh" => "Chinese",
        "cs" => "Czech",
        "cy" => "Welsh",
        "da" => "Danish",
        "de" => "German",
        "el" => "Greek",
        "en" => "English",
        "es" => "Spanish",
        "et" => "Estonian",
        "eu" => "Basque",
        "fa" => "Persian",
        "fi" => "Finnish",
        "fil" => "Filipino",
        "fr" => "French",
        "ga" => "Irish",
        "gl" => "Galician",
        "gu" => "Gujarati",
        "he" | "iw" => "Hebrew",
        "hi" => "Hindi",
        "hr" => "Croatian",
        "hu" => "Hungarian",
        "id" => "Indonesian",
        "is" => "Icelandic",
        "it" => "Italian",
        "ja" => "Japanese",
        "jv" => "Javanese",
        "kn" => "Kannada",
        "ko" => "Korean",
        "lt" => "Lithuanian",
        "lv" => "Latvian",
        "ml" => "Malayalam",
        "mr" => "Marathi",
        "ms" => "Malay",
        "nb" => "Norwegian Bokmål",
        "nl" => "Dutch",
        "pa" => "Punjabi",
        "pl" => "Polish",
        "pt" => "Portuguese",
        "ro" => "Romanian",
        "ru" => "Russian",
        "sk" => "Slovak",
        "sl" => "Slovenian",
        "sr" => "Serbian",
        "su" => "Sundanese",
        "sv" => "Swedish",
        "sw" => "Swahili",
        "ta" => "Tamil",
        "te" => "Telugu",
        "th" => "Thai",
        "tr" => "Turkish",
        "uk" => "Ukrainian",
        "ur" => "Urdu",
        "vi" => "Vietnamese",
        "yue" => "Cantonese",
        _ => code, // Return the code itself if unknown
    }
    .to_string()
}

/// Extracts accent/region from a language code (e.g., "US" from "en-US").
fn extract_accent_from_code(code: &str) -> String {
    let parts: Vec<&str> = code.split('-').collect();
    if parts.len() >= 2 {
        // Map region codes to readable names
        match parts[1].to_uppercase().as_str() {
            "US" => "American",
            "GB" => "British",
            "AU" => "Australian",
            "IN" => "Indian",
            "CA" => "Canadian",
            "IE" => "Irish",
            "NZ" => "New Zealand",
            "ZA" => "South African",
            "ES" => "Spain",
            "MX" => "Mexican",
            "AR" => "Argentinian",
            "CL" => "Chilean",
            "CO" => "Colombian",
            "PE" => "Peruvian",
            "VE" => "Venezuelan",
            "BR" => "Brazilian",
            "PT" => "Portuguese",
            "FR" => "French",
            "BE" => "Belgian",
            "CH" => "Swiss",
            "DE" => "German",
            "AT" => "Austrian",
            "IT" => "Italian",
            "CN" => "Mainland China",
            "TW" => "Taiwanese",
            "HK" => "Hong Kong",
            "JP" => "Japanese",
            "KR" => "Korean",
            "RU" => "Russian",
            "UA" => "Ukrainian",
            "PL" => "Polish",
            "NL" => "Dutch",
            "SE" => "Swedish",
            "NO" => "Norwegian",
            "DK" => "Danish",
            "FI" => "Finnish",
            "TR" => "Turkish",
            "SA" => "Saudi",
            "EG" => "Egyptian",
            "IL" => "Israeli",
            "PH" => "Filipino",
            "ID" => "Indonesian",
            "MY" => "Malaysian",
            "TH" => "Thai",
            "VN" => "Vietnamese",
            _ => parts[1],
        }
        .to_string()
    } else {
        "Standard".to_string()
    }
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

// Helper function to fetch voices from Google TTS API
async fn fetch_google_voices(
    credentials: &str,
) -> Result<Vec<Voice>, Box<dyn std::error::Error + Send + Sync>> {
    // Create credential source and auth client
    let credential_source = CredentialSource::from_api_key(credentials);
    let auth_client = GoogleAuthClient::new(credential_source, &[GOOGLE_CLOUD_PLATFORM_SCOPE])?;

    // Get OAuth2 token
    let token = auth_client.get_token().await?;

    let client = reqwest::Client::new();

    let response = client
        .get("https://texttospeech.googleapis.com/v1/voices")
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let error_body = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(format!("Google TTS API error ({}): {}", status, error_body).into());
    }

    let google_response: GoogleVoicesResponse = response.json().await?;

    let voices = google_response
        .voices
        .unwrap_or_default()
        .into_iter()
        .map(|voice| {
            // Use first language code for language and accent
            let primary_lang = voice.language_codes.first().cloned().unwrap_or_default();
            let language = language_code_to_name(&primary_lang);
            let accent = extract_accent_from_code(&primary_lang);

            // Map SSML gender to our format
            let gender = match voice.ssml_gender.as_deref() {
                Some("MALE") => "Male".to_string(),
                Some("FEMALE") => "Female".to_string(),
                Some("NEUTRAL") => "Neutral".to_string(),
                _ => "Unknown".to_string(),
            };

            // Extract display name from voice name (e.g., "en-US-Wavenet-D" -> "Wavenet D")
            let display_name = voice
                .name
                .split('-')
                .skip(2) // Skip language and region
                .collect::<Vec<&str>>()
                .join(" ");
            let display_name = if display_name.is_empty() {
                voice.name.clone()
            } else {
                display_name
            };

            Voice {
                id: voice.name,
                sample: String::new(), // Google TTS doesn't provide sample URLs
                name: display_name,
                accent,
                gender,
                language,
            }
        })
        .collect();

    Ok(voices)
}

// Helper function to fetch voices from Azure TTS API
async fn fetch_azure_voices(
    subscription_key: &str,
    region: &str,
) -> Result<Vec<Voice>, Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::Client::new();

    // Azure TTS voices list endpoint
    let url = format!(
        "https://{}.tts.speech.microsoft.com/cognitiveservices/voices/list",
        region
    );

    let response = client
        .get(&url)
        .header("Ocp-Apim-Subscription-Key", subscription_key)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let error_body = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(format!("Azure TTS API error ({}): {}", status, error_body).into());
    }

    let azure_voices: Vec<AzureVoice> = response.json().await?;

    let voices = azure_voices
        .into_iter()
        .map(|voice| {
            let language = language_code_to_name(&voice.locale);
            let accent = extract_accent_from_code(&voice.locale);

            Voice {
                id: voice.short_name,
                sample: String::new(), // Azure doesn't provide sample URLs in this API
                name: voice.display_name,
                accent,
                gender: voice.gender,
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

    // Fetch ElevenLabs voices - skip if not configured
    if let Ok(api_key) = state.config.get_api_key("elevenlabs") {
        match fetch_elevenlabs_voices(&api_key).await {
            Ok(voices) => {
                voices_response.insert("elevenlabs".to_string(), voices);
            }
            Err(e) => {
                tracing::warn!("Failed to fetch ElevenLabs voices: {}", e);
            }
        }
    } else {
        tracing::debug!("ElevenLabs API key not configured, skipping");
    }

    // Fetch Deepgram voices - skip if not configured
    if let Ok(api_key) = state.config.get_api_key("deepgram") {
        match fetch_deepgram_voices(&api_key).await {
            Ok(voices) => {
                voices_response.insert("deepgram".to_string(), voices);
            }
            Err(e) => {
                tracing::warn!("Failed to fetch Deepgram voices: {}", e);
            }
        }
    } else {
        tracing::debug!("Deepgram API key not configured, skipping");
    }

    // Fetch Google TTS voices - skip if not configured
    // Note: Google returns empty string for ADC which is valid
    if let Ok(credentials) = state.config.get_api_key("google") {
        match fetch_google_voices(&credentials).await {
            Ok(voices) => {
                voices_response.insert("google".to_string(), voices);
            }
            Err(e) => {
                tracing::warn!("Failed to fetch Google TTS voices: {}", e);
            }
        }
    } else {
        tracing::debug!("Google credentials not configured, skipping");
    }

    // Fetch Azure TTS voices - skip if not configured
    if let Ok(subscription_key) = state.config.get_api_key("microsoft-azure") {
        let region = state.config.get_azure_speech_region();
        match fetch_azure_voices(&subscription_key, &region).await {
            Ok(voices) => {
                voices_response.insert("azure".to_string(), voices);
            }
            Err(e) => {
                tracing::warn!("Failed to fetch Azure TTS voices: {}", e);
            }
        }
    } else {
        tracing::debug!("Azure Speech credentials not configured, skipping");
    }

    Ok(Json(voices_response))
}
