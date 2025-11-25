use std::time::Duration;

use crate::core::stt::base::STTConfig;
use crate::core::stt::google::GoogleSTTConfig;

#[test]
fn test_google_stt_config_default() {
    let config = GoogleSTTConfig::default();

    assert!(config.project_id.is_empty());
    assert_eq!(config.location, "global");
    assert!(config.recognizer_id.is_none());
    assert!(config.interim_results);
    assert!(config.enable_voice_activity_events);
    assert!(config.speech_start_timeout.is_none());
    assert!(config.speech_end_timeout.is_none());
    assert!(!config.single_utterance);
    assert_eq!(config.base.model, "latest_long");
}

#[test]
fn test_recognizer_path_with_default_recognizer() {
    let config = GoogleSTTConfig {
        project_id: "test-project".to_string(),
        location: "global".to_string(),
        recognizer_id: None,
        ..Default::default()
    };

    assert_eq!(
        config.recognizer_path(),
        "projects/test-project/locations/global/recognizers/_"
    );
}

#[test]
fn test_recognizer_path_with_custom_recognizer() {
    let config = GoogleSTTConfig {
        project_id: "my-project".to_string(),
        location: "us".to_string(),
        recognizer_id: Some("custom-recognizer".to_string()),
        ..Default::default()
    };

    assert_eq!(
        config.recognizer_path(),
        "projects/my-project/locations/us/recognizers/custom-recognizer"
    );
}

#[test]
fn test_recognizer_path_with_eu_location() {
    let config = GoogleSTTConfig {
        project_id: "eu-project".to_string(),
        location: "eu".to_string(),
        recognizer_id: Some("eu-recognizer".to_string()),
        ..Default::default()
    };

    assert_eq!(
        config.recognizer_path(),
        "projects/eu-project/locations/eu/recognizers/eu-recognizer"
    );
}

#[test]
fn test_google_encoding_mapping() {
    let mut config = GoogleSTTConfig::default();

    config.base.encoding = "linear16".to_string();
    assert_eq!(config.google_encoding(), "LINEAR16");

    config.base.encoding = "pcm".to_string();
    assert_eq!(config.google_encoding(), "LINEAR16");

    config.base.encoding = "flac".to_string();
    assert_eq!(config.google_encoding(), "FLAC");

    config.base.encoding = "mulaw".to_string();
    assert_eq!(config.google_encoding(), "MULAW");

    config.base.encoding = "ogg_opus".to_string();
    assert_eq!(config.google_encoding(), "OGG_OPUS");

    config.base.encoding = "opus".to_string();
    assert_eq!(config.google_encoding(), "OGG_OPUS");

    config.base.encoding = "webm_opus".to_string();
    assert_eq!(config.google_encoding(), "WEBM_OPUS");

    config.base.encoding = "unknown".to_string();
    assert_eq!(config.google_encoding(), "LINEAR16");
}

#[test]
fn test_config_with_timeouts() {
    let config = GoogleSTTConfig {
        project_id: "test-project".to_string(),
        speech_start_timeout: Some(Duration::from_secs(5)),
        speech_end_timeout: Some(Duration::from_millis(1500)),
        ..Default::default()
    };

    assert_eq!(config.speech_start_timeout, Some(Duration::from_secs(5)));
    assert_eq!(config.speech_end_timeout, Some(Duration::from_millis(1500)));
}

#[test]
fn test_config_serialization() {
    let config = GoogleSTTConfig {
        project_id: "test-project".to_string(),
        location: "us".to_string(),
        recognizer_id: Some("my-recognizer".to_string()),
        interim_results: true,
        enable_voice_activity_events: false,
        speech_start_timeout: Some(Duration::from_millis(5000)),
        speech_end_timeout: None,
        single_utterance: true,
        ..Default::default()
    };

    let json = serde_json::to_string(&config).expect("Failed to serialize");
    let deserialized: GoogleSTTConfig = serde_json::from_str(&json).expect("Failed to deserialize");

    assert_eq!(deserialized.project_id, "test-project");
    assert_eq!(deserialized.location, "us");
    assert_eq!(
        deserialized.recognizer_id,
        Some("my-recognizer".to_string())
    );
    assert!(deserialized.interim_results);
    assert!(!deserialized.enable_voice_activity_events);
    assert_eq!(
        deserialized.speech_start_timeout,
        Some(Duration::from_millis(5000))
    );
    assert!(deserialized.speech_end_timeout.is_none());
    assert!(deserialized.single_utterance);
}

#[test]
fn test_config_clone() {
    let config = GoogleSTTConfig {
        project_id: "clone-test".to_string(),
        recognizer_id: Some("test".to_string()),
        ..Default::default()
    };

    let cloned = config.clone();
    assert_eq!(cloned.project_id, config.project_id);
    assert_eq!(cloned.recognizer_id, config.recognizer_id);
}

#[test]
fn test_recognizer_path_with_different_locations() {
    for location in &[
        "global",
        "us",
        "eu",
        "asia-southeast1",
        "northamerica-northeast1",
    ] {
        let config = GoogleSTTConfig {
            project_id: "test-project".to_string(),
            location: location.to_string(),
            recognizer_id: None,
            ..Default::default()
        };

        let path = config.recognizer_path();
        assert!(
            path.contains(location),
            "Path should contain location '{location}': {path}"
        );
        assert_eq!(
            path,
            format!("projects/test-project/locations/{location}/recognizers/_")
        );
    }
}

#[test]
fn test_recognizer_path_with_special_characters_in_recognizer_id() {
    let config = GoogleSTTConfig {
        project_id: "my-project".to_string(),
        location: "global".to_string(),
        recognizer_id: Some("my-recognizer-v1.0".to_string()),
        ..Default::default()
    };

    let path = config.recognizer_path();
    assert_eq!(
        path,
        "projects/my-project/locations/global/recognizers/my-recognizer-v1.0"
    );
}

#[test]
fn test_google_stt_config_with_all_options() {
    let config = GoogleSTTConfig {
        base: STTConfig {
            provider: "google".to_string(),
            api_key: String::new(),
            language: "ja-JP".to_string(),
            sample_rate: 48000,
            channels: 2,
            punctuation: false,
            encoding: "flac".to_string(),
            model: "chirp".to_string(),
        },
        project_id: "test-project".to_string(),
        location: "asia-northeast1".to_string(),
        recognizer_id: Some("japanese-recognizer".to_string()),
        interim_results: false,
        enable_voice_activity_events: false,
        speech_start_timeout: Some(Duration::from_secs(10)),
        speech_end_timeout: Some(Duration::from_secs(2)),
        single_utterance: true,
    };

    assert_eq!(config.base.language, "ja-JP");
    assert_eq!(config.base.sample_rate, 48000);
    assert_eq!(config.base.channels, 2);
    assert!(!config.base.punctuation);
    assert_eq!(config.base.encoding, "flac");
    assert_eq!(config.base.model, "chirp");
    assert!(!config.interim_results);
    assert!(!config.enable_voice_activity_events);
    assert!(config.single_utterance);
    assert_eq!(config.speech_start_timeout, Some(Duration::from_secs(10)));
    assert_eq!(config.speech_end_timeout, Some(Duration::from_secs(2)));
    assert_eq!(config.google_encoding(), "FLAC");
}

#[test]
fn test_google_encoding_case_insensitive() {
    let mut config = GoogleSTTConfig::default();

    config.base.encoding = "LINEAR16".to_string();
    assert_eq!(config.google_encoding(), "LINEAR16");

    config.base.encoding = "Linear16".to_string();
    assert_eq!(config.google_encoding(), "LINEAR16");

    config.base.encoding = "FLAC".to_string();
    assert_eq!(config.google_encoding(), "FLAC");

    config.base.encoding = "Flac".to_string();
    assert_eq!(config.google_encoding(), "FLAC");
}

#[test]
fn test_google_encoding_amr_variants() {
    let mut config = GoogleSTTConfig::default();

    config.base.encoding = "amr".to_string();
    assert_eq!(config.google_encoding(), "AMR");

    config.base.encoding = "amr_wb".to_string();
    assert_eq!(config.google_encoding(), "AMR_WB");

    config.base.encoding = "amr-wb".to_string();
    assert_eq!(config.google_encoding(), "AMR_WB");

    config.base.encoding = "AMR_WB".to_string();
    assert_eq!(config.google_encoding(), "AMR_WB");
}

#[test]
fn test_config_serialization_roundtrip() {
    let original = GoogleSTTConfig {
        base: STTConfig {
            provider: "google".to_string(),
            api_key: "test-key".to_string(),
            language: "en-GB".to_string(),
            sample_rate: 44100,
            channels: 2,
            punctuation: true,
            encoding: "flac".to_string(),
            model: "telephony".to_string(),
        },
        project_id: "roundtrip-project".to_string(),
        location: "europe-west1".to_string(),
        recognizer_id: Some("test-recognizer".to_string()),
        interim_results: false,
        enable_voice_activity_events: true,
        speech_start_timeout: Some(Duration::from_millis(3500)),
        speech_end_timeout: Some(Duration::from_millis(750)),
        single_utterance: true,
    };

    let json = serde_json::to_string(&original).expect("Serialization failed");
    let deserialized: GoogleSTTConfig =
        serde_json::from_str(&json).expect("Deserialization failed");

    assert_eq!(deserialized.project_id, original.project_id);
    assert_eq!(deserialized.location, original.location);
    assert_eq!(deserialized.recognizer_id, original.recognizer_id);
    assert_eq!(deserialized.interim_results, original.interim_results);
    assert_eq!(
        deserialized.enable_voice_activity_events,
        original.enable_voice_activity_events
    );
    assert_eq!(
        deserialized.speech_start_timeout,
        original.speech_start_timeout
    );
    assert_eq!(deserialized.speech_end_timeout, original.speech_end_timeout);
    assert_eq!(deserialized.single_utterance, original.single_utterance);
    assert_eq!(deserialized.base.language, original.base.language);
}

#[test]
fn test_config_deserialization_without_optional_fields() {
    let json = r#"{
            "base": {
                "provider": "google",
                "api_key": "",
                "language": "en-US",
                "sample_rate": 16000,
                "channels": 1,
                "punctuation": true,
                "encoding": "linear16",
                "model": "latest_long"
            },
            "project_id": "minimal-project",
            "location": "global",
            "recognizer_id": null,
            "interim_results": true,
            "enable_voice_activity_events": true,
            "single_utterance": false
        }"#;

    let config: GoogleSTTConfig = serde_json::from_str(json).expect("Deserialization failed");

    assert_eq!(config.project_id, "minimal-project");
    assert!(config.speech_start_timeout.is_none());
    assert!(config.speech_end_timeout.is_none());
}
