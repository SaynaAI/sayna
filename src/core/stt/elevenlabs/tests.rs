//! Unit tests for ElevenLabs STT implementation.
//!
//! Tests are organized into logical sections:
//! - Configuration tests (audio format, commit strategy, region, config building)
//! - Message tests (serialization, deserialization, parsing)
//! - Client tests (URL building, message handling, BaseSTT trait)

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::protocol::Message;

use super::client::{ConnectionState, ElevenLabsSTT};
use super::config::{CommitStrategy, ElevenLabsAudioFormat, ElevenLabsRegion, ElevenLabsSTTConfig};
use super::messages::{
    CommittedTranscript, CommittedTranscriptWithTimestamps, ElevenLabsMessage, ElevenLabsSTTError,
    EndOfStream, InputAudioChunk, PartialTranscript, SessionStarted, WordTiming,
};
use crate::core::stt::base::{BaseSTT, STTConfig, STTError, STTResult};

// =============================================================================
// Audio Format Tests
// =============================================================================

mod audio_format_tests {
    use super::*;

    #[test]
    fn test_as_str() {
        assert_eq!(ElevenLabsAudioFormat::Pcm8000.as_str(), "pcm_8000");
        assert_eq!(ElevenLabsAudioFormat::Pcm16000.as_str(), "pcm_16000");
        assert_eq!(ElevenLabsAudioFormat::Pcm22050.as_str(), "pcm_22050");
        assert_eq!(ElevenLabsAudioFormat::Pcm24000.as_str(), "pcm_24000");
        assert_eq!(ElevenLabsAudioFormat::Pcm44100.as_str(), "pcm_44100");
        assert_eq!(ElevenLabsAudioFormat::Pcm48000.as_str(), "pcm_48000");
        assert_eq!(ElevenLabsAudioFormat::Ulaw8000.as_str(), "ulaw_8000");
    }

    #[test]
    fn test_sample_rate() {
        assert_eq!(ElevenLabsAudioFormat::Pcm8000.sample_rate(), 8000);
        assert_eq!(ElevenLabsAudioFormat::Pcm16000.sample_rate(), 16000);
        assert_eq!(ElevenLabsAudioFormat::Pcm22050.sample_rate(), 22050);
        assert_eq!(ElevenLabsAudioFormat::Pcm24000.sample_rate(), 24000);
        assert_eq!(ElevenLabsAudioFormat::Pcm44100.sample_rate(), 44100);
        assert_eq!(ElevenLabsAudioFormat::Pcm48000.sample_rate(), 48000);
        assert_eq!(ElevenLabsAudioFormat::Ulaw8000.sample_rate(), 8000);
    }

    #[test]
    fn test_from_sample_rate() {
        assert_eq!(
            ElevenLabsAudioFormat::from_sample_rate(8000),
            ElevenLabsAudioFormat::Pcm8000
        );
        assert_eq!(
            ElevenLabsAudioFormat::from_sample_rate(16000),
            ElevenLabsAudioFormat::Pcm16000
        );
        assert_eq!(
            ElevenLabsAudioFormat::from_sample_rate(22050),
            ElevenLabsAudioFormat::Pcm22050
        );
        assert_eq!(
            ElevenLabsAudioFormat::from_sample_rate(24000),
            ElevenLabsAudioFormat::Pcm24000
        );
        assert_eq!(
            ElevenLabsAudioFormat::from_sample_rate(44100),
            ElevenLabsAudioFormat::Pcm44100
        );
        assert_eq!(
            ElevenLabsAudioFormat::from_sample_rate(48000),
            ElevenLabsAudioFormat::Pcm48000
        );
    }

    #[test]
    fn test_from_sample_rate_unknown() {
        // Unknown rates default to 16kHz
        assert_eq!(
            ElevenLabsAudioFormat::from_sample_rate(11025),
            ElevenLabsAudioFormat::Pcm16000
        );
        assert_eq!(
            ElevenLabsAudioFormat::from_sample_rate(96000),
            ElevenLabsAudioFormat::Pcm16000
        );
        assert_eq!(
            ElevenLabsAudioFormat::from_sample_rate(0),
            ElevenLabsAudioFormat::Pcm16000
        );
    }

    #[test]
    fn test_default() {
        assert_eq!(
            ElevenLabsAudioFormat::default(),
            ElevenLabsAudioFormat::Pcm16000
        );
    }
}

// =============================================================================
// Commit Strategy Tests
// =============================================================================

mod commit_strategy_tests {
    use super::*;

    #[test]
    fn test_as_str() {
        assert_eq!(CommitStrategy::Manual.as_str(), "manual");
        assert_eq!(CommitStrategy::Vad.as_str(), "vad");
    }

    #[test]
    fn test_default() {
        assert_eq!(CommitStrategy::default(), CommitStrategy::Vad);
    }
}

// =============================================================================
// Region Tests
// =============================================================================

mod region_tests {
    use super::*;

    #[test]
    fn test_websocket_urls() {
        assert_eq!(
            ElevenLabsRegion::Default.websocket_base_url(),
            "wss://api.elevenlabs.io"
        );
        assert_eq!(
            ElevenLabsRegion::Us.websocket_base_url(),
            "wss://api.us.elevenlabs.io"
        );
        assert_eq!(
            ElevenLabsRegion::Eu.websocket_base_url(),
            "wss://api.eu.residency.elevenlabs.io"
        );
        assert_eq!(
            ElevenLabsRegion::India.websocket_base_url(),
            "wss://api.in.residency.elevenlabs.io"
        );
    }

    #[test]
    fn test_host() {
        assert_eq!(ElevenLabsRegion::Default.host(), "api.elevenlabs.io");
        assert_eq!(ElevenLabsRegion::Us.host(), "api.us.elevenlabs.io");
        assert_eq!(
            ElevenLabsRegion::Eu.host(),
            "api.eu.residency.elevenlabs.io"
        );
        assert_eq!(
            ElevenLabsRegion::India.host(),
            "api.in.residency.elevenlabs.io"
        );
    }

    #[test]
    fn test_default() {
        assert_eq!(ElevenLabsRegion::default(), ElevenLabsRegion::Default);
    }
}

// =============================================================================
// Configuration Tests
// =============================================================================

mod config_tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ElevenLabsSTTConfig::default();
        assert_eq!(config.model_id, "scribe_v2_realtime");
        assert_eq!(config.audio_format, ElevenLabsAudioFormat::Pcm16000);
        assert_eq!(config.commit_strategy, CommitStrategy::Vad);
        assert!(!config.include_timestamps);
        // VAD defaults are set for responsive end-of-speech detection
        assert_eq!(config.vad_silence_threshold_secs, Some(0.5));
        assert!(config.vad_threshold.is_none());
        assert_eq!(config.min_speech_duration_ms, Some(50));
        assert_eq!(config.min_silence_duration_ms, Some(300));
        assert!(!config.enable_logging);
        assert_eq!(config.region, ElevenLabsRegion::Default);
    }

    #[test]
    fn test_build_websocket_url_minimal() {
        // Create config with no VAD params to test minimal URL
        let config = ElevenLabsSTTConfig {
            base: STTConfig {
                language: String::new(),
                ..Default::default()
            },
            vad_silence_threshold_secs: None,
            vad_threshold: None,
            min_speech_duration_ms: None,
            min_silence_duration_ms: None,
            ..Default::default()
        };

        let url = config.build_websocket_url();
        assert!(url.starts_with("wss://api.elevenlabs.io/v1/speech-to-text/realtime?"));
        assert!(url.contains("model_id=scribe_v2_realtime"));
        assert!(url.contains("audio_format=pcm_16000"));
        assert!(url.contains("commit_strategy=vad"));
        assert!(!url.contains("language_code="));
        assert!(!url.contains("vad_silence_threshold_secs"));
    }

    #[test]
    fn test_build_websocket_url_full() {
        let config = ElevenLabsSTTConfig {
            base: STTConfig {
                language: "en".to_string(),
                ..Default::default()
            },
            model_id: "scribe_v1_experimental".to_string(),
            audio_format: ElevenLabsAudioFormat::Pcm24000,
            commit_strategy: CommitStrategy::Manual,
            include_timestamps: true,
            vad_silence_threshold_secs: Some(0.5),
            vad_threshold: Some(0.3),
            min_speech_duration_ms: Some(100),
            min_silence_duration_ms: Some(500),
            enable_logging: true,
            region: ElevenLabsRegion::Us,
        };

        let url = config.build_websocket_url();
        assert!(url.starts_with("wss://api.us.elevenlabs.io/v1/speech-to-text/realtime?"));
        assert!(url.contains("model_id=scribe_v1_experimental"));
        assert!(url.contains("audio_format=pcm_24000"));
        assert!(url.contains("commit_strategy=manual"));
        assert!(url.contains("language_code=en"));
        assert!(url.contains("include_timestamps=true"));
        assert!(url.contains("vad_silence_threshold_secs=0.5"));
        assert!(url.contains("vad_threshold=0.3"));
        assert!(url.contains("min_speech_duration_ms=100"));
        assert!(url.contains("min_silence_duration_ms=500"));
        assert!(url.contains("enable_logging=true"));
    }

    #[test]
    fn test_from_base() {
        let base = STTConfig {
            api_key: "test_key".to_string(),
            sample_rate: 24000,
            language: "fr".to_string(),
            ..Default::default()
        };

        let config = ElevenLabsSTTConfig::from_base(base);
        assert_eq!(config.audio_format, ElevenLabsAudioFormat::Pcm24000);
        assert_eq!(config.base.language, "fr");
        assert_eq!(config.base.api_key, "test_key");
    }
}

// =============================================================================
// Input Message Tests
// =============================================================================

mod input_message_tests {
    use super::*;

    #[test]
    fn test_input_audio_chunk_new() {
        let chunk = InputAudioChunk::new("SGVsbG8gV29ybGQ=".to_string());
        assert_eq!(chunk.message_type, "input_audio_chunk");
        assert_eq!(chunk.audio_base_64, "SGVsbG8gV29ybGQ=");
        assert!(chunk.commit.is_none());
        assert!(chunk.sample_rate.is_none());
    }

    #[test]
    fn test_input_audio_chunk_with_commit() {
        let chunk = InputAudioChunk::new("SGVsbG8=".to_string()).with_commit(true);
        assert_eq!(chunk.commit, Some(true));
    }

    #[test]
    fn test_input_audio_chunk_with_sample_rate() {
        let chunk = InputAudioChunk::new("SGVsbG8=".to_string()).with_sample_rate(16000);
        assert_eq!(chunk.sample_rate, Some(16000));
    }

    #[test]
    fn test_input_audio_chunk_serialization() {
        let chunk = InputAudioChunk::new("SGVsbG8=".to_string());
        let json = serde_json::to_string(&chunk).unwrap();
        assert!(json.contains(r#""message_type":"input_audio_chunk""#));
        assert!(json.contains(r#""audio_base_64":"SGVsbG8=""#));
        assert!(!json.contains("commit"));
        assert!(!json.contains("sample_rate"));
    }

    #[test]
    fn test_input_audio_chunk_serialization_with_options() {
        let chunk = InputAudioChunk::new("SGVsbG8=".to_string())
            .with_commit(true)
            .with_sample_rate(16000);
        let json = serde_json::to_string(&chunk).unwrap();
        assert!(json.contains(r#""commit":true"#));
        assert!(json.contains(r#""sample_rate":16000"#));
    }

    #[test]
    fn test_end_of_stream() {
        let eos = EndOfStream::default();
        assert_eq!(eos.message_type, "eos");

        let json = serde_json::to_string(&eos).unwrap();
        assert!(json.contains(r#""message_type":"eos""#));
    }
}

// =============================================================================
// Message Parsing Tests
// =============================================================================

mod message_parsing_tests {
    use super::*;

    #[test]
    fn test_session_started() {
        let json = r#"{"message_type": "session_started", "session_id": "abc123"}"#;
        let msg = ElevenLabsMessage::parse(json).unwrap();
        match msg {
            ElevenLabsMessage::SessionStarted(s) => {
                assert_eq!(s.session_id, "abc123");
                assert!(s.config.is_none());
            }
            _ => panic!("Expected SessionStarted"),
        }
    }

    #[test]
    fn test_partial_transcript() {
        let json = r#"{"message_type": "partial_transcript", "text": "hello"}"#;
        let msg = ElevenLabsMessage::parse(json).unwrap();
        assert!(!msg.is_final_transcript());
        match msg {
            ElevenLabsMessage::PartialTranscript(p) => {
                assert_eq!(p.text, "hello");
            }
            _ => panic!("Expected PartialTranscript"),
        }
    }

    #[test]
    fn test_committed_transcript() {
        let json = r#"{"message_type": "committed_transcript", "text": "hello world"}"#;
        let msg = ElevenLabsMessage::parse(json).unwrap();
        match &msg {
            ElevenLabsMessage::CommittedTranscript(c) => {
                assert_eq!(c.text, "hello world");
            }
            _ => panic!("Expected CommittedTranscript"),
        }
        assert!(msg.is_final_transcript());
    }

    #[test]
    fn test_committed_with_timestamps() {
        let json = r#"{
            "message_type": "committed_transcript_with_timestamps",
            "text": "hello world",
            "language_code": "en",
            "words": [
                {"text": "hello", "start": 0.0, "end": 0.5, "type": "word"},
                {"text": "world", "start": 0.6, "end": 1.0, "type": "word"}
            ]
        }"#;
        let msg = ElevenLabsMessage::parse(json).unwrap();
        match &msg {
            ElevenLabsMessage::CommittedTranscriptWithTimestamps(c) => {
                assert_eq!(c.text, "hello world");
                assert_eq!(c.language_code, Some("en".to_string()));
                assert_eq!(c.words.len(), 2);
                assert_eq!(c.words[0].text, "hello");
                assert_eq!(c.words[0].start, 0.0);
                assert_eq!(c.words[0].end, 0.5);
                assert_eq!(c.words[1].text, "world");
            }
            _ => panic!("Expected CommittedTranscriptWithTimestamps"),
        }
        assert!(msg.is_final_transcript());
    }

    #[test]
    fn test_error() {
        let json = r#"{"message_type": "error", "error": "Invalid audio format"}"#;
        let msg = ElevenLabsMessage::parse(json).unwrap();
        match &msg {
            ElevenLabsMessage::Error(e) => {
                assert_eq!(e.error, "Invalid audio format");
            }
            _ => panic!("Expected Error"),
        }
        assert!(msg.is_error());
    }

    #[test]
    fn test_auth_error() {
        let json = r#"{"message_type": "auth_error", "error": "Invalid API key"}"#;
        let msg = ElevenLabsMessage::parse(json).unwrap();
        match &msg {
            ElevenLabsMessage::Error(e) => {
                assert_eq!(e.message_type, "auth_error");
                assert_eq!(e.error, "Invalid API key");
            }
            _ => panic!("Expected Error"),
        }
    }

    #[test]
    fn test_quota_error() {
        let json = r#"{"message_type": "quota_exceeded_error", "error": "Rate limit exceeded"}"#;
        let msg = ElevenLabsMessage::parse(json).unwrap();
        match &msg {
            ElevenLabsMessage::Error(e) => {
                assert_eq!(e.message_type, "quota_exceeded_error");
            }
            _ => panic!("Expected Error"),
        }
    }

    #[test]
    fn test_unknown() {
        let json = r#"{"message_type": "future_message_type", "data": "something"}"#;
        let msg = ElevenLabsMessage::parse(json).unwrap();
        match msg {
            ElevenLabsMessage::Unknown(s) => {
                assert!(s.contains("future_message_type"));
            }
            _ => panic!("Expected Unknown"),
        }
    }

    #[test]
    fn test_invalid_json() {
        let json = "not valid json";
        let result = ElevenLabsMessage::parse(json);
        assert!(result.is_err());
    }
}

// =============================================================================
// Word Timing Tests
// =============================================================================

mod word_timing_tests {
    use super::*;

    #[test]
    fn test_full_deserialization() {
        let json = r#"{
            "text": "hello",
            "start": 0.0,
            "end": 0.5,
            "type": "word",
            "speaker_id": "speaker_1",
            "logprob": -0.123,
            "characters": ["h", "e", "l", "l", "o"]
        }"#;
        let word: WordTiming = serde_json::from_str(json).unwrap();
        assert_eq!(word.text, "hello");
        assert_eq!(word.start, 0.0);
        assert_eq!(word.end, 0.5);
        assert_eq!(word.word_type, "word");
        assert_eq!(word.speaker_id, Some("speaker_1".to_string()));
        assert_eq!(word.logprob, Some(-0.123));
        assert_eq!(
            word.characters,
            Some(vec![
                "h".to_string(),
                "e".to_string(),
                "l".to_string(),
                "l".to_string(),
                "o".to_string()
            ])
        );
    }

    #[test]
    fn test_minimal_deserialization() {
        let json = r#"{"text": "hello", "start": 0.0, "end": 0.5, "type": "word"}"#;
        let word: WordTiming = serde_json::from_str(json).unwrap();
        assert_eq!(word.text, "hello");
        assert!(word.speaker_id.is_none());
        assert!(word.logprob.is_none());
        assert!(word.characters.is_none());
    }
}

// =============================================================================
// Message Helper Method Tests
// =============================================================================

mod message_helper_tests {
    use super::*;

    #[test]
    fn test_is_error() {
        let error_msg = ElevenLabsMessage::Error(ElevenLabsSTTError {
            message_type: "error".to_string(),
            error: "test".to_string(),
        });
        assert!(error_msg.is_error());

        let partial = ElevenLabsMessage::PartialTranscript(PartialTranscript {
            message_type: "partial_transcript".to_string(),
            text: "test".to_string(),
        });
        assert!(!partial.is_error());
    }

    #[test]
    fn test_is_final_transcript() {
        let committed = ElevenLabsMessage::CommittedTranscript(CommittedTranscript {
            message_type: "committed_transcript".to_string(),
            text: "test".to_string(),
        });
        assert!(committed.is_final_transcript());

        let committed_with_ts = ElevenLabsMessage::CommittedTranscriptWithTimestamps(
            CommittedTranscriptWithTimestamps {
                message_type: "committed_transcript_with_timestamps".to_string(),
                text: "test".to_string(),
                language_code: None,
                words: vec![],
            },
        );
        assert!(committed_with_ts.is_final_transcript());

        let partial = ElevenLabsMessage::PartialTranscript(PartialTranscript {
            message_type: "partial_transcript".to_string(),
            text: "test".to_string(),
        });
        assert!(!partial.is_final_transcript());

        let session = ElevenLabsMessage::SessionStarted(SessionStarted {
            message_type: "session_started".to_string(),
            session_id: "test".to_string(),
            config: None,
        });
        assert!(!session.is_final_transcript());
    }
}

// =============================================================================
// ElevenLabsSTT Struct Tests
// =============================================================================

mod client_tests {
    use super::*;

    #[test]
    fn test_default() {
        let stt = ElevenLabsSTT::default();
        assert!(stt.config.is_none());
        assert!(!stt.is_ready()); // Checks ws_sender internally
        assert!(stt.get_session_id().is_none());
        assert!(matches!(stt.state, ConnectionState::Disconnected));
    }

    #[test]
    fn test_url_building() {
        let stt = ElevenLabsSTT::default();
        let config = ElevenLabsSTTConfig {
            base: STTConfig {
                api_key: "test_key".to_string(),
                language: "en-US".to_string(),
                sample_rate: 16000,
                ..Default::default()
            },
            model_id: "scribe_v1".to_string(),
            audio_format: ElevenLabsAudioFormat::Pcm16000,
            commit_strategy: CommitStrategy::Vad,
            include_timestamps: true,
            vad_silence_threshold_secs: Some(0.5),
            vad_threshold: Some(0.3),
            min_speech_duration_ms: Some(100),
            min_silence_duration_ms: Some(500),
            enable_logging: false,
            region: ElevenLabsRegion::Default,
        };

        let url = stt.build_websocket_url(&config).unwrap();

        assert!(url.starts_with("wss://api.elevenlabs.io/v1/speech-to-text/realtime?"));
        assert!(url.contains("model_id=scribe_v1"));
        assert!(url.contains("audio_format=pcm_16000"));
        assert!(url.contains("language_code=en"));
        assert!(url.contains("commit_strategy=vad"));
        assert!(url.contains("include_timestamps=true"));
        assert!(url.contains("vad_silence_threshold_secs=0.5"));
        assert!(url.contains("vad_threshold=0.3"));
        assert!(url.contains("min_speech_duration_ms=100"));
        assert!(url.contains("min_silence_duration_ms=500"));
    }

    #[test]
    fn test_url_building_minimal_config() {
        let stt = ElevenLabsSTT::default();
        // Create config with no VAD params for minimal URL test
        let config = ElevenLabsSTTConfig {
            vad_silence_threshold_secs: None,
            vad_threshold: None,
            min_speech_duration_ms: None,
            min_silence_duration_ms: None,
            ..Default::default()
        };

        let url = stt.build_websocket_url(&config).unwrap();

        assert!(url.starts_with("wss://api.elevenlabs.io/v1/speech-to-text/realtime?"));
        assert!(url.contains("model_id=scribe_v2_realtime"));
        assert!(url.contains("audio_format=pcm_16000"));
        assert!(!url.contains("vad_silence_threshold_secs"));
    }

    #[test]
    fn test_url_building_us_region() {
        let stt = ElevenLabsSTT::default();
        let mut config = ElevenLabsSTTConfig::default();
        config.region = ElevenLabsRegion::Us;

        let url = stt.build_websocket_url(&config).unwrap();

        assert!(url.starts_with("wss://api.us.elevenlabs.io/"));
    }

    #[test]
    fn test_url_building_eu_region() {
        let stt = ElevenLabsSTT::default();
        let mut config = ElevenLabsSTTConfig::default();
        config.region = ElevenLabsRegion::Eu;

        let url = stt.build_websocket_url(&config).unwrap();

        assert!(url.starts_with("wss://api.eu.residency.elevenlabs.io/"));
    }

    #[test]
    fn test_get_host_from_region() {
        assert_eq!(
            ElevenLabsSTT::get_host_from_region(&ElevenLabsRegion::Default),
            "api.elevenlabs.io"
        );
        assert_eq!(
            ElevenLabsSTT::get_host_from_region(&ElevenLabsRegion::Us),
            "api.us.elevenlabs.io"
        );
        assert_eq!(
            ElevenLabsSTT::get_host_from_region(&ElevenLabsRegion::Eu),
            "api.eu.residency.elevenlabs.io"
        );
        assert_eq!(
            ElevenLabsSTT::get_host_from_region(&ElevenLabsRegion::India),
            "api.in.residency.elevenlabs.io"
        );
    }

    #[test]
    fn test_encode_audio() {
        let audio_data = vec![0u8, 1, 2, 3, 4, 5];
        let encoded = ElevenLabsSTT::encode_audio_for_transmission(&audio_data);
        assert_eq!(encoded, "AAECAwQF");
    }

    #[test]
    fn test_encode_audio_empty() {
        let audio_data: Vec<u8> = vec![];
        let encoded = ElevenLabsSTT::encode_audio_for_transmission(&audio_data);
        assert_eq!(encoded, "");
    }

    #[test]
    fn test_encode_audio_large() {
        use base64::prelude::*;

        let audio_data: Vec<u8> = (0..=255).collect();
        let encoded = ElevenLabsSTT::encode_audio_for_transmission(&audio_data);

        let decoded = BASE64_STANDARD.decode(&encoded).unwrap();
        assert_eq!(decoded, audio_data);
    }
}

// =============================================================================
// WebSocket Message Handler Tests
// =============================================================================

mod websocket_handler_tests {
    use super::*;

    #[tokio::test]
    async fn test_handle_session_started() {
        let (result_tx, _result_rx) = mpsc::unbounded_channel::<STTResult>();
        let mut session_id: Option<String> = None;

        let json = r#"{"message_type": "session_started", "session_id": "test-session-123"}"#;
        let message = Message::Text(json.into());

        let result = ElevenLabsSTT::handle_websocket_message(message, &result_tx, &mut session_id);

        assert!(result.is_ok());
        assert_eq!(session_id, Some("test-session-123".to_string()));
    }

    #[tokio::test]
    async fn test_handle_partial_transcript() {
        let (result_tx, mut result_rx) = mpsc::unbounded_channel::<STTResult>();
        let mut session_id: Option<String> = None;

        let json = r#"{"message_type": "partial_transcript", "text": "hello world"}"#;
        let message = Message::Text(json.into());

        let result = ElevenLabsSTT::handle_websocket_message(message, &result_tx, &mut session_id);

        assert!(result.is_ok());

        let stt_result = tokio::time::timeout(Duration::from_millis(100), result_rx.recv())
            .await
            .expect("Should receive result")
            .expect("Channel should have result");

        assert_eq!(stt_result.transcript, "hello world");
        assert!(!stt_result.is_final);
        assert!(!stt_result.is_speech_final);
    }

    #[tokio::test]
    async fn test_handle_committed_transcript() {
        let (result_tx, mut result_rx) = mpsc::unbounded_channel::<STTResult>();
        let mut session_id: Option<String> = None;

        let json = r#"{"message_type": "committed_transcript", "text": "hello world final"}"#;
        let message = Message::Text(json.into());

        let result = ElevenLabsSTT::handle_websocket_message(message, &result_tx, &mut session_id);

        assert!(result.is_ok());

        let stt_result = tokio::time::timeout(Duration::from_millis(100), result_rx.recv())
            .await
            .expect("Should receive result")
            .expect("Channel should have result");

        assert_eq!(stt_result.transcript, "hello world final");
        assert!(stt_result.is_final);
        assert!(stt_result.is_speech_final);
        assert_eq!(stt_result.confidence, 1.0);
    }

    #[tokio::test]
    async fn test_handle_auth_error() {
        let (result_tx, _result_rx) = mpsc::unbounded_channel::<STTResult>();
        let mut session_id: Option<String> = None;

        let json = r#"{"message_type": "auth_error", "error": "Invalid API key"}"#;
        let message = Message::Text(json.into());

        let result = ElevenLabsSTT::handle_websocket_message(message, &result_tx, &mut session_id);

        assert!(result.is_err());
        match result {
            Err(STTError::AuthenticationFailed(msg)) => {
                assert!(msg.contains("Invalid API key"));
            }
            _ => panic!("Expected AuthenticationFailed error"),
        }
    }

    #[tokio::test]
    async fn test_handle_quota_exceeded() {
        let (result_tx, _result_rx) = mpsc::unbounded_channel::<STTResult>();
        let mut session_id: Option<String> = None;

        let json = r#"{"message_type": "quota_exceeded_error", "error": "Rate limit exceeded"}"#;
        let message = Message::Text(json.into());

        let result = ElevenLabsSTT::handle_websocket_message(message, &result_tx, &mut session_id);

        assert!(result.is_err());
        match result {
            Err(STTError::ProviderError(msg)) => {
                assert!(msg.contains("Quota exceeded"));
            }
            _ => panic!("Expected ProviderError"),
        }
    }

    #[tokio::test]
    async fn test_handle_empty_partial() {
        let (result_tx, mut result_rx) = mpsc::unbounded_channel::<STTResult>();
        let mut session_id: Option<String> = None;

        let json = r#"{"message_type": "partial_transcript", "text": ""}"#;
        let message = Message::Text(json.into());

        let result = ElevenLabsSTT::handle_websocket_message(message, &result_tx, &mut session_id);

        assert!(result.is_ok());

        // Should timeout because no result was sent
        let recv_result = tokio::time::timeout(Duration::from_millis(50), result_rx.recv()).await;
        assert!(recv_result.is_err());
    }

    #[tokio::test]
    async fn test_handle_committed_with_timestamps() {
        let (result_tx, mut result_rx) = mpsc::unbounded_channel::<STTResult>();
        let mut session_id: Option<String> = None;

        let json = r#"{
            "message_type": "committed_transcript_with_timestamps",
            "text": "hello world",
            "language_code": "en",
            "words": [
                {"text": "hello", "start": 0.0, "end": 0.5, "type": "word", "logprob": -0.1},
                {"text": "world", "start": 0.5, "end": 1.0, "type": "word", "logprob": -0.2}
            ]
        }"#;
        let message = Message::Text(json.into());

        let result = ElevenLabsSTT::handle_websocket_message(message, &result_tx, &mut session_id);

        assert!(result.is_ok());

        let stt_result = tokio::time::timeout(Duration::from_millis(100), result_rx.recv())
            .await
            .expect("Should receive result")
            .expect("Channel should have result");

        assert_eq!(stt_result.transcript, "hello world");
        assert!(stt_result.is_final);
        assert!(stt_result.is_speech_final);
        assert!(stt_result.confidence > 0.0 && stt_result.confidence <= 1.0);
    }

    #[tokio::test]
    async fn test_handle_committed_with_timestamps_no_logprob() {
        let (result_tx, mut result_rx) = mpsc::unbounded_channel::<STTResult>();
        let mut session_id: Option<String> = None;

        let json = r#"{
            "message_type": "committed_transcript_with_timestamps",
            "text": "hello world",
            "language_code": "en",
            "words": [
                {"text": "hello", "start": 0.0, "end": 0.5, "type": "word"},
                {"text": "world", "start": 0.5, "end": 1.0, "type": "word"}
            ]
        }"#;
        let message = Message::Text(json.into());

        let result = ElevenLabsSTT::handle_websocket_message(message, &result_tx, &mut session_id);

        assert!(result.is_ok());

        let stt_result = tokio::time::timeout(Duration::from_millis(100), result_rx.recv())
            .await
            .expect("Should receive result")
            .expect("Channel should have result");

        // When no logprobs, confidence defaults to 1.0
        assert_eq!(stt_result.confidence, 1.0);
    }

    #[tokio::test]
    async fn test_handle_generic_error() {
        let (result_tx, _result_rx) = mpsc::unbounded_channel::<STTResult>();
        let mut session_id: Option<String> = None;

        let json = r#"{"message_type": "error", "error": "Generic error message"}"#;
        let message = Message::Text(json.into());

        let result = ElevenLabsSTT::handle_websocket_message(message, &result_tx, &mut session_id);

        assert!(result.is_err());
        match result {
            Err(STTError::ProviderError(msg)) => {
                assert_eq!(msg, "Generic error message");
            }
            _ => panic!("Expected ProviderError"),
        }
    }

    #[tokio::test]
    async fn test_handle_unknown_message_type() {
        let (result_tx, mut result_rx) = mpsc::unbounded_channel::<STTResult>();
        let mut session_id: Option<String> = None;

        let json = r#"{"message_type": "some_future_type", "data": "something"}"#;
        let message = Message::Text(json.into());

        let result = ElevenLabsSTT::handle_websocket_message(message, &result_tx, &mut session_id);

        assert!(result.is_ok());

        let recv_result = tokio::time::timeout(Duration::from_millis(50), result_rx.recv()).await;
        assert!(recv_result.is_err());
    }

    #[tokio::test]
    async fn test_handle_close_message() {
        let (result_tx, _result_rx) = mpsc::unbounded_channel::<STTResult>();
        let mut session_id: Option<String> = None;

        let message = Message::Close(None);

        let result = ElevenLabsSTT::handle_websocket_message(message, &result_tx, &mut session_id);

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_ping_message() {
        let (result_tx, _result_rx) = mpsc::unbounded_channel::<STTResult>();
        let mut session_id: Option<String> = None;

        let message = Message::Ping(vec![1, 2, 3].into());

        let result = ElevenLabsSTT::handle_websocket_message(message, &result_tx, &mut session_id);

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_pong_message() {
        let (result_tx, _result_rx) = mpsc::unbounded_channel::<STTResult>();
        let mut session_id: Option<String> = None;

        let message = Message::Pong(vec![1, 2, 3].into());

        let result = ElevenLabsSTT::handle_websocket_message(message, &result_tx, &mut session_id);

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_binary_message() {
        let (result_tx, _result_rx) = mpsc::unbounded_channel::<STTResult>();
        let mut session_id: Option<String> = None;

        let message = Message::Binary(vec![1, 2, 3].into());

        let result = ElevenLabsSTT::handle_websocket_message(message, &result_tx, &mut session_id);

        assert!(result.is_ok());
    }
}

// =============================================================================
// BaseSTT Trait Implementation Tests
// =============================================================================

mod base_stt_tests {
    use super::*;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn test_new_valid_config() {
        let config = STTConfig {
            api_key: "test_key".to_string(),
            language: "en-US".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            model: "".to_string(),
            provider: "elevenlabs".to_string(),
        };

        let stt = <ElevenLabsSTT as BaseSTT>::new(config);
        assert!(stt.is_ok());

        let stt = stt.unwrap();
        assert!(!stt.is_ready());
        assert_eq!(
            stt.get_provider_info(),
            "ElevenLabs STT Real-Time WebSocket"
        );
    }

    #[tokio::test]
    async fn test_new_empty_api_key() {
        let config = STTConfig {
            api_key: String::new(),
            ..Default::default()
        };

        let result = <ElevenLabsSTT as BaseSTT>::new(config);
        assert!(result.is_err());

        match result {
            Err(STTError::AuthenticationFailed(msg)) => {
                assert!(msg.contains("API key is required"));
            }
            _ => panic!("Expected AuthenticationFailed error"),
        }
    }

    #[tokio::test]
    async fn test_get_config() {
        let config = STTConfig {
            api_key: "test_key".to_string(),
            language: "es".to_string(),
            sample_rate: 24000,
            ..Default::default()
        };

        let stt = <ElevenLabsSTT as BaseSTT>::new(config).unwrap();
        let retrieved_config = stt.get_config();

        assert!(retrieved_config.is_some());
        let retrieved_config = retrieved_config.unwrap();
        assert_eq!(retrieved_config.api_key, "test_key");
        assert_eq!(retrieved_config.language, "es");
        assert_eq!(retrieved_config.sample_rate, 24000);
    }

    #[tokio::test]
    async fn test_is_ready_before_connect() {
        let config = STTConfig {
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let stt = <ElevenLabsSTT as BaseSTT>::new(config).unwrap();
        assert!(!stt.is_ready());
    }

    #[tokio::test]
    async fn test_send_audio_not_connected() {
        let config = STTConfig {
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let mut stt = <ElevenLabsSTT as BaseSTT>::new(config).unwrap();
        let result = stt.send_audio(vec![0u8; 100]).await;

        assert!(result.is_err());
        match result {
            Err(STTError::ConnectionFailed(_)) => {}
            _ => panic!("Expected ConnectionFailed error"),
        }
    }

    #[tokio::test]
    async fn test_callback_registration() {
        let config = STTConfig {
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let mut stt = <ElevenLabsSTT as BaseSTT>::new(config).unwrap();

        // Register result callback
        let result_callback = Arc::new(|_result: STTResult| {
            Box::pin(async move {}) as Pin<Box<dyn Future<Output = ()> + Send>>
        });
        let result = stt.on_result(result_callback).await;
        assert!(result.is_ok());

        // Register error callback
        let error_callback = Arc::new(|_error: STTError| {
            Box::pin(async move {}) as Pin<Box<dyn Future<Output = ()> + Send>>
        });
        let result = stt.on_error(error_callback).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_audio_format_from_sample_rate_via_new() {
        let configs = vec![
            (8000, ElevenLabsAudioFormat::Pcm8000),
            (16000, ElevenLabsAudioFormat::Pcm16000),
            (24000, ElevenLabsAudioFormat::Pcm24000),
            (44100, ElevenLabsAudioFormat::Pcm44100),
            (48000, ElevenLabsAudioFormat::Pcm48000),
        ];

        for (sample_rate, expected_format) in configs {
            let config = STTConfig {
                api_key: "test_key".to_string(),
                sample_rate,
                ..Default::default()
            };

            let stt = <ElevenLabsSTT as BaseSTT>::new(config).unwrap();
            assert_eq!(
                stt.config.as_ref().unwrap().audio_format,
                expected_format,
                "Sample rate {} should map to {:?}",
                sample_rate,
                expected_format
            );
        }
    }

    #[tokio::test]
    async fn test_get_session_id_not_connected() {
        let config = STTConfig {
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let stt = <ElevenLabsSTT as BaseSTT>::new(config).unwrap();
        assert!(stt.get_session_id().is_none());
    }

    #[tokio::test]
    async fn test_provider_info() {
        let config = STTConfig {
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let stt = <ElevenLabsSTT as BaseSTT>::new(config).unwrap();
        assert_eq!(
            stt.get_provider_info(),
            "ElevenLabs STT Real-Time WebSocket"
        );
    }

    #[tokio::test]
    async fn test_disconnect_when_not_connected() {
        let config = STTConfig {
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let mut stt = <ElevenLabsSTT as BaseSTT>::new(config).unwrap();
        let result = stt.disconnect().await;
        assert!(result.is_ok());
        assert!(!stt.is_ready());
    }

    #[tokio::test]
    async fn test_new_sets_default_elevenlabs_config() {
        let config = STTConfig {
            api_key: "test_key".to_string(),
            language: "en".to_string(),
            sample_rate: 16000,
            ..Default::default()
        };

        let stt = <ElevenLabsSTT as BaseSTT>::new(config).unwrap();

        let elevenlabs_config = stt.config.as_ref().unwrap();
        assert_eq!(elevenlabs_config.model_id, "scribe_v2_realtime");
        assert_eq!(
            elevenlabs_config.audio_format,
            ElevenLabsAudioFormat::Pcm16000
        );
        assert_eq!(elevenlabs_config.commit_strategy, CommitStrategy::Vad);
        assert!(!elevenlabs_config.include_timestamps);
        // VAD defaults are set for responsive end-of-speech detection
        assert_eq!(elevenlabs_config.vad_silence_threshold_secs, Some(0.5));
        assert!(elevenlabs_config.vad_threshold.is_none());
        assert_eq!(elevenlabs_config.min_speech_duration_ms, Some(50));
        assert_eq!(elevenlabs_config.min_silence_duration_ms, Some(300));
        assert!(!elevenlabs_config.enable_logging);
        assert_eq!(elevenlabs_config.region, ElevenLabsRegion::Default);
    }

    #[tokio::test]
    async fn test_config_language_preserved() {
        let config = STTConfig {
            api_key: "test_key".to_string(),
            language: "fr-FR".to_string(),
            sample_rate: 24000,
            channels: 2,
            punctuation: false,
            encoding: "opus".to_string(),
            model: "custom".to_string(),
            provider: "elevenlabs".to_string(),
        };

        let stt = <ElevenLabsSTT as BaseSTT>::new(config).unwrap();

        let retrieved_config = stt.get_config().unwrap();
        assert_eq!(retrieved_config.language, "fr-FR");
        assert_eq!(retrieved_config.sample_rate, 24000);
        assert_eq!(retrieved_config.channels, 2);
        assert!(!retrieved_config.punctuation);
        assert_eq!(retrieved_config.encoding, "opus");
    }

    #[tokio::test]
    async fn test_state_transitions() {
        let config = STTConfig {
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let stt = <ElevenLabsSTT as BaseSTT>::new(config).unwrap();

        assert!(matches!(stt.state, ConnectionState::Disconnected));
        assert!(!stt.is_ready());
    }

    #[tokio::test]
    async fn test_multiple_callback_registrations() {
        let config = STTConfig {
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let mut stt = <ElevenLabsSTT as BaseSTT>::new(config).unwrap();

        // Register first callback
        let counter1 = Arc::new(AtomicUsize::new(0));
        let counter1_clone = counter1.clone();
        let callback1 = Arc::new(move |_result: STTResult| {
            counter1_clone.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {}) as Pin<Box<dyn Future<Output = ()> + Send>>
        });
        stt.on_result(callback1).await.unwrap();

        // Register second callback (should replace first)
        let counter2 = Arc::new(AtomicUsize::new(0));
        let counter2_clone = counter2.clone();
        let callback2 = Arc::new(move |_result: STTResult| {
            counter2_clone.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {}) as Pin<Box<dyn Future<Output = ()> + Send>>
        });
        stt.on_result(callback2).await.unwrap();

        assert!(stt.result_callback.lock().await.is_some());
    }
}
