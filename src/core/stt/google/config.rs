use std::time::Duration;

use crate::core::stt::base::STTConfig;

/// Configuration specific to Google Speech-to-Text v2 API.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GoogleSTTConfig {
    pub base: STTConfig,
    pub project_id: String,
    pub location: String,
    pub recognizer_id: Option<String>,
    pub interim_results: bool,
    pub enable_voice_activity_events: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "optional_duration_serde"
    )]
    pub speech_start_timeout: Option<Duration>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "optional_duration_serde"
    )]
    pub speech_end_timeout: Option<Duration>,
    pub single_utterance: bool,
}

impl Default for GoogleSTTConfig {
    fn default() -> Self {
        Self {
            base: STTConfig {
                model: "latest_long".to_string(),
                ..STTConfig::default()
            },
            project_id: String::new(),
            location: "global".to_string(),
            recognizer_id: None,
            interim_results: true,
            enable_voice_activity_events: true,
            speech_start_timeout: None,
            speech_end_timeout: None,
            single_utterance: false,
        }
    }
}

impl GoogleSTTConfig {
    pub fn recognizer_path(&self) -> String {
        let recognizer = self.recognizer_id.as_deref().unwrap_or("_");
        format!(
            "projects/{}/locations/{}/recognizers/{}",
            self.project_id, self.location, recognizer
        )
    }

    /// Maps the base config encoding to Google's encoding name.
    pub fn google_encoding(&self) -> &'static str {
        match self.base.encoding.to_lowercase().as_str() {
            "linear16" | "pcm" => "LINEAR16",
            "flac" => "FLAC",
            "mulaw" | "ulaw" => "MULAW",
            "amr" => "AMR",
            "amr_wb" | "amr-wb" => "AMR_WB",
            "ogg_opus" | "ogg-opus" | "opus" => "OGG_OPUS",
            "webm_opus" | "webm-opus" => "WEBM_OPUS",
            _ => "LINEAR16",
        }
    }
}

/// Serde helper module for optional Duration serialization.
mod optional_duration_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(value: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(duration) => {
                let millis = duration.as_millis() as u64;
                millis.serialize(serializer)
            }
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt: Option<u64> = Option::deserialize(deserializer)?;
        Ok(opt.map(Duration::from_millis))
    }
}
