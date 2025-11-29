//! Unit tests for Cartesia STT implementation.
//!
//! Tests are organized into logical sections:
//! - Configuration tests (audio encoding, config building)
//! - Message tests (serialization, deserialization, parsing)
//! - Client tests (BaseSTT trait implementation)

use super::client::CartesiaSTT;
use super::config::{CartesiaAudioEncoding, CartesiaSTTConfig};
use super::messages::{
    CartesiaCommand, CartesiaMessage, CartesiaSTTError, CartesiaTranscript, CartesiaWord,
    DoneMessage,
};
use crate::core::stt::base::{BaseSTT, STTConfig, STTError};

// =============================================================================
// Audio Encoding Tests
// =============================================================================

mod audio_encoding_tests {
    use super::*;

    #[test]
    fn test_as_str() {
        assert_eq!(CartesiaAudioEncoding::PcmS16le.as_str(), "pcm_s16le");
    }

    #[test]
    fn test_default() {
        assert_eq!(
            CartesiaAudioEncoding::default(),
            CartesiaAudioEncoding::PcmS16le
        );
    }
}

// =============================================================================
// Configuration Tests
// =============================================================================

mod config_tests {
    use super::super::config::{SUPPORTED_SAMPLE_RATES, is_sample_rate_supported};
    use super::*;

    #[test]
    fn test_default_config() {
        let config = CartesiaSTTConfig::default();
        assert_eq!(config.model, "ink-whisper");
        assert_eq!(config.encoding, CartesiaAudioEncoding::PcmS16le);
        assert!(config.min_volume.is_none());
        assert!(config.max_silence_duration_secs.is_none());
        assert!(config.cartesia_version.is_none());
    }

    #[test]
    fn test_default_config_base_values() {
        let config = CartesiaSTTConfig::default();
        assert_eq!(config.base.sample_rate, 16000);
        assert_eq!(config.base.language, "en-US"); // Default from STTConfig
        assert_eq!(config.base.channels, 1);
    }

    #[test]
    fn test_build_websocket_url_minimal() {
        let config = CartesiaSTTConfig {
            base: STTConfig {
                language: "en".to_string(),
                sample_rate: 16000,
                ..Default::default()
            },
            ..Default::default()
        };

        let url = config.build_websocket_url("test_api_key");
        assert!(url.starts_with("wss://api.cartesia.ai/stt/websocket?"));
        assert!(url.contains("api_key=test_api_key"));
        assert!(url.contains("model=ink-whisper"));
        assert!(url.contains("language=en"));
        assert!(url.contains("encoding=pcm_s16le"));
        assert!(url.contains("sample_rate=16000"));
        assert!(!url.contains("min_volume"));
        assert!(!url.contains("max_silence_duration_secs"));
        assert!(!url.contains("cartesia_version"));
    }

    #[test]
    fn test_build_websocket_url_full() {
        let config = CartesiaSTTConfig {
            base: STTConfig {
                language: "fr".to_string(),
                sample_rate: 24000,
                ..Default::default()
            },
            model: "ink-whisper".to_string(),
            encoding: CartesiaAudioEncoding::PcmS16le,
            min_volume: Some(0.1),
            max_silence_duration_secs: Some(0.5),
            cartesia_version: Some("2025-01-15".to_string()),
        };

        let url = config.build_websocket_url("my_api_key");
        assert!(url.contains("api_key=my_api_key"));
        assert!(url.contains("model=ink-whisper"));
        assert!(url.contains("language=fr"));
        assert!(url.contains("encoding=pcm_s16le"));
        assert!(url.contains("sample_rate=24000"));
        assert!(url.contains("min_volume=0.1"));
        assert!(url.contains("max_silence_duration_secs=0.5"));
        assert!(url.contains("cartesia_version=2025-01-15"));
    }

    #[test]
    fn test_from_base() {
        let base = STTConfig {
            api_key: "test_key".to_string(),
            sample_rate: 24000,
            language: "es".to_string(),
            ..Default::default()
        };

        let config = CartesiaSTTConfig::from_base(base);
        assert_eq!(config.base.language, "es");
        assert_eq!(config.base.sample_rate, 24000);
        assert_eq!(config.base.api_key, "test_key");
        assert_eq!(config.model, "ink-whisper");
    }

    // =========================================================================
    // Sample Rate Validation Tests
    // =========================================================================

    #[test]
    fn test_supported_sample_rates_constant() {
        assert_eq!(SUPPORTED_SAMPLE_RATES.len(), 6);
        assert!(SUPPORTED_SAMPLE_RATES.contains(&8000));
        assert!(SUPPORTED_SAMPLE_RATES.contains(&16000));
        assert!(SUPPORTED_SAMPLE_RATES.contains(&22050));
        assert!(SUPPORTED_SAMPLE_RATES.contains(&24000));
        assert!(SUPPORTED_SAMPLE_RATES.contains(&44100));
        assert!(SUPPORTED_SAMPLE_RATES.contains(&48000));
    }

    #[test]
    fn test_is_sample_rate_supported_valid() {
        for rate in [8000, 16000, 22050, 24000, 44100, 48000] {
            assert!(
                is_sample_rate_supported(rate),
                "Sample rate {} should be supported",
                rate
            );
        }
    }

    #[test]
    fn test_is_sample_rate_supported_invalid() {
        for rate in [0, 1000, 11025, 12000, 32000, 96000, 192000] {
            assert!(
                !is_sample_rate_supported(rate),
                "Sample rate {} should not be supported",
                rate
            );
        }
    }

    #[test]
    fn test_validate_supported_sample_rates() {
        for rate in [8000, 16000, 22050, 24000, 44100, 48000] {
            let config = CartesiaSTTConfig {
                base: STTConfig {
                    sample_rate: rate,
                    api_key: "test".to_string(),
                    language: "en".to_string(),
                    ..Default::default()
                },
                ..Default::default()
            };
            assert!(
                config.validate().is_ok(),
                "Validation should pass for sample rate {}",
                rate
            );
        }
    }

    #[test]
    fn test_validate_unsupported_sample_rate() {
        let config = CartesiaSTTConfig {
            base: STTConfig {
                sample_rate: 12000, // Unsupported
                api_key: "test".to_string(),
                language: "en".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };

        let result = config.validate();
        assert!(result.is_err());

        match result {
            Err(STTError::ConfigurationError(msg)) => {
                assert!(msg.contains("Unsupported sample rate"));
                assert!(msg.contains("12000"));
            }
            _ => panic!("Expected ConfigurationError"),
        }
    }

    #[test]
    fn test_validate_empty_api_key() {
        let config = CartesiaSTTConfig {
            base: STTConfig {
                api_key: String::new(),
                language: "en".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };

        let result = config.validate();
        assert!(result.is_err());

        match result {
            Err(STTError::ConfigurationError(msg)) => {
                assert!(msg.contains("API key is required"));
            }
            _ => panic!("Expected ConfigurationError"),
        }
    }

    #[test]
    fn test_validate_empty_language() {
        let config = CartesiaSTTConfig {
            base: STTConfig {
                api_key: "test".to_string(),
                language: String::new(),
                ..Default::default()
            },
            ..Default::default()
        };

        let result = config.validate();
        assert!(result.is_err());

        match result {
            Err(STTError::ConfigurationError(msg)) => {
                assert!(msg.contains("Language code is required"));
            }
            _ => panic!("Expected ConfigurationError"),
        }
    }

    #[test]
    fn test_validate_min_volume_valid_range() {
        for volume in [0.0, 0.1, 0.5, 0.9, 1.0] {
            let config = CartesiaSTTConfig {
                base: STTConfig {
                    api_key: "test".to_string(),
                    language: "en".to_string(),
                    ..Default::default()
                },
                min_volume: Some(volume),
                ..Default::default()
            };
            assert!(
                config.validate().is_ok(),
                "min_volume {} should be valid",
                volume
            );
        }
    }

    #[test]
    fn test_validate_min_volume_out_of_range() {
        for volume in [-0.1, 1.1, 2.0, -1.0] {
            let config = CartesiaSTTConfig {
                base: STTConfig {
                    api_key: "test".to_string(),
                    language: "en".to_string(),
                    ..Default::default()
                },
                min_volume: Some(volume),
                ..Default::default()
            };

            let result = config.validate();
            assert!(result.is_err(), "min_volume {} should be invalid", volume);

            match result {
                Err(STTError::ConfigurationError(msg)) => {
                    assert!(msg.contains("min_volume must be between 0.0 and 1.0"));
                }
                _ => panic!("Expected ConfigurationError"),
            }
        }
    }

    #[test]
    fn test_validate_max_silence_duration_valid() {
        for duration in [0.1, 0.5, 1.0, 2.0, 10.0] {
            let config = CartesiaSTTConfig {
                base: STTConfig {
                    api_key: "test".to_string(),
                    language: "en".to_string(),
                    ..Default::default()
                },
                max_silence_duration_secs: Some(duration),
                ..Default::default()
            };
            assert!(
                config.validate().is_ok(),
                "max_silence_duration_secs {} should be valid",
                duration
            );
        }
    }

    #[test]
    fn test_validate_max_silence_duration_invalid() {
        for duration in [0.0, -0.1, -1.0] {
            let config = CartesiaSTTConfig {
                base: STTConfig {
                    api_key: "test".to_string(),
                    language: "en".to_string(),
                    ..Default::default()
                },
                max_silence_duration_secs: Some(duration),
                ..Default::default()
            };

            let result = config.validate();
            assert!(
                result.is_err(),
                "max_silence_duration_secs {} should be invalid",
                duration
            );

            match result {
                Err(STTError::ConfigurationError(msg)) => {
                    assert!(msg.contains("max_silence_duration_secs must be positive"));
                }
                _ => panic!("Expected ConfigurationError"),
            }
        }
    }

    #[test]
    fn test_validate_all_fields_valid() {
        let config = CartesiaSTTConfig {
            base: STTConfig {
                api_key: "test_key".to_string(),
                language: "en".to_string(),
                sample_rate: 16000,
                ..Default::default()
            },
            model: "ink-whisper".to_string(),
            encoding: CartesiaAudioEncoding::PcmS16le,
            min_volume: Some(0.1),
            max_silence_duration_secs: Some(0.4),
            cartesia_version: Some("2025-01-15".to_string()),
        };

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_websocket_base_url_constant() {
        assert_eq!(
            CartesiaSTTConfig::WEBSOCKET_BASE_URL,
            "wss://api.cartesia.ai/stt/websocket"
        );
    }
}

// =============================================================================
// Message Serialization Tests
// =============================================================================

mod message_serialization_tests {
    use super::*;

    #[test]
    fn test_done_message() {
        let done = DoneMessage::default();
        assert_eq!(done.message_type, "done");

        let json = serde_json::to_string(&done).unwrap();
        assert!(json.contains(r#""type":"done""#));
    }

    #[test]
    fn test_command_finalize_serialization() {
        let cmd = CartesiaCommand::Finalize;
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(json, "\"finalize\"");
    }

    #[test]
    fn test_command_done_serialization() {
        let cmd = CartesiaCommand::Done;
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(json, "\"done\"");
    }
}

// =============================================================================
// Message Parsing Tests
// =============================================================================

mod message_parsing_tests {
    use super::*;

    #[test]
    fn test_transcript_interim() {
        let json = r#"{"type": "transcript", "text": "hello", "is_final": false}"#;
        let msg = CartesiaMessage::parse(json).unwrap();
        assert!(msg.is_transcript());
        assert!(!msg.is_final_transcript());
        match msg {
            CartesiaMessage::Transcript(t) => {
                assert_eq!(t.text, "hello");
                assert!(!t.is_final);
            }
            _ => panic!("Expected Transcript message"),
        }
    }

    #[test]
    fn test_transcript_final() {
        let json = r#"{"type": "transcript", "text": "hello world", "is_final": true}"#;
        let msg = CartesiaMessage::parse(json).unwrap();
        assert!(msg.is_final_transcript());
        assert!(msg.is_transcript());
        match msg {
            CartesiaMessage::Transcript(t) => {
                assert_eq!(t.text, "hello world");
                assert!(t.is_final);
            }
            _ => panic!("Expected Transcript message"),
        }
    }

    #[test]
    fn test_transcript_with_words() {
        let json = r#"{
            "type": "transcript",
            "text": "Hello world",
            "is_final": true,
            "words": [
                {"word": "Hello", "start": 0.0, "end": 0.5},
                {"word": "world", "start": 0.6, "end": 1.2}
            ]
        }"#;
        let msg = CartesiaMessage::parse(json).unwrap();
        match msg {
            CartesiaMessage::Transcript(t) => {
                assert!(t.words.is_some());
                let words = t.words.unwrap();
                assert_eq!(words.len(), 2);
                assert_eq!(words[0].word, "Hello");
                assert_eq!(words[1].word, "world");
            }
            _ => panic!("Expected Transcript message"),
        }
    }

    #[test]
    fn test_flush_done_message() {
        let json = r#"{"type": "flush_done"}"#;
        let msg = CartesiaMessage::parse(json).unwrap();
        assert!(matches!(msg, CartesiaMessage::FlushDone));
        assert!(!msg.is_transcript());
        assert!(!msg.is_error());
    }

    #[test]
    fn test_done_message() {
        let json = r#"{"type": "done"}"#;
        let msg = CartesiaMessage::parse(json).unwrap();
        assert!(matches!(msg, CartesiaMessage::Done));
    }

    #[test]
    fn test_error_message() {
        let json = r#"{"type": "error", "message": "Invalid audio format"}"#;
        let msg = CartesiaMessage::parse(json).unwrap();
        assert!(msg.is_error());
        match msg {
            CartesiaMessage::Error(e) => {
                assert_eq!(e.message, "Invalid audio format");
            }
            _ => panic!("Expected Error message"),
        }
    }

    #[test]
    fn test_unknown_message() {
        let json = r#"{"type": "future_type", "data": "something"}"#;
        let msg = CartesiaMessage::parse(json).unwrap();
        match msg {
            CartesiaMessage::Unknown(s) => {
                assert!(s.contains("future_type"));
            }
            _ => panic!("Expected Unknown message"),
        }
    }

    #[test]
    fn test_invalid_json() {
        let json = "not valid json";
        let result = CartesiaMessage::parse(json);
        assert!(result.is_err());
    }
}

// =============================================================================
// Message Helper Method Tests
// =============================================================================

mod message_helper_tests {
    use super::*;

    #[test]
    fn test_is_error() {
        let error_msg = CartesiaMessage::Error(CartesiaSTTError {
            message: "test error".to_string(),
        });
        assert!(error_msg.is_error());

        let transcript = CartesiaMessage::Transcript(CartesiaTranscript {
            text: "test".to_string(),
            is_final: false,
            words: None,
        });
        assert!(!transcript.is_error());
    }

    #[test]
    fn test_is_final_transcript() {
        let final_msg = CartesiaMessage::Transcript(CartesiaTranscript {
            text: "test".to_string(),
            is_final: true,
            words: None,
        });
        assert!(final_msg.is_final_transcript());

        let interim = CartesiaMessage::Transcript(CartesiaTranscript {
            text: "test".to_string(),
            is_final: false,
            words: None,
        });
        assert!(!interim.is_final_transcript());
    }

    #[test]
    fn test_is_transcript() {
        let interim = CartesiaMessage::Transcript(CartesiaTranscript {
            text: "test".to_string(),
            is_final: false,
            words: None,
        });
        assert!(interim.is_transcript());

        let final_msg = CartesiaMessage::Transcript(CartesiaTranscript {
            text: "test".to_string(),
            is_final: true,
            words: None,
        });
        assert!(final_msg.is_transcript());

        assert!(!CartesiaMessage::Done.is_transcript());
        assert!(!CartesiaMessage::FlushDone.is_transcript());
    }

    #[test]
    fn test_to_stt_result_final() {
        let msg = CartesiaMessage::Transcript(CartesiaTranscript {
            text: "Hello world".to_string(),
            is_final: true,
            words: None,
        });

        let result = msg.to_stt_result().unwrap();
        assert_eq!(result.transcript, "Hello world");
        assert!(result.is_final);
        assert!(result.is_speech_final);
        assert_eq!(result.confidence, 1.0);
    }

    #[test]
    fn test_to_stt_result_interim() {
        let msg = CartesiaMessage::Transcript(CartesiaTranscript {
            text: "Hel".to_string(),
            is_final: false,
            words: None,
        });

        let result = msg.to_stt_result().unwrap();
        assert_eq!(result.transcript, "Hel");
        assert!(!result.is_final);
        assert!(!result.is_speech_final);
        assert_eq!(result.confidence, 0.0);
    }

    #[test]
    fn test_to_stt_result_non_transcript() {
        assert!(CartesiaMessage::Done.to_stt_result().is_none());
        assert!(CartesiaMessage::FlushDone.to_stt_result().is_none());
        assert!(
            CartesiaMessage::Error(CartesiaSTTError {
                message: "error".to_string()
            })
            .to_stt_result()
            .is_none()
        );
    }

    #[test]
    fn test_as_transcript() {
        let msg = CartesiaMessage::Transcript(CartesiaTranscript {
            text: "test".to_string(),
            is_final: true,
            words: None,
        });
        assert!(msg.as_transcript().is_some());
        assert!(CartesiaMessage::Done.as_transcript().is_none());
    }

    #[test]
    fn test_as_error() {
        let msg = CartesiaMessage::Error(CartesiaSTTError {
            message: "test error".to_string(),
        });
        assert!(msg.as_error().is_some());
        assert_eq!(msg.as_error().unwrap().message, "test error");
        assert!(CartesiaMessage::Done.as_error().is_none());
    }
}

// =============================================================================
// CartesiaWord Tests
// =============================================================================

mod word_tests {
    use super::*;

    #[test]
    fn test_word_deserialization() {
        let json = r#"{"word": "Hello", "start": 0.0, "end": 0.5}"#;
        let word: CartesiaWord = serde_json::from_str(json).unwrap();

        assert_eq!(word.word, "Hello");
        assert_eq!(word.start, 0.0);
        assert_eq!(word.end, 0.5);
    }

    #[test]
    fn test_word_array_deserialization() {
        let json = r#"[
            {"word": "Hello", "start": 0.0, "end": 0.5},
            {"word": "world", "start": 0.6, "end": 1.2}
        ]"#;
        let words: Vec<CartesiaWord> = serde_json::from_str(json).unwrap();

        assert_eq!(words.len(), 2);
        assert_eq!(words[0].word, "Hello");
        assert_eq!(words[1].word, "world");
    }

    #[test]
    fn test_word_equality() {
        let word1 = CartesiaWord {
            word: "test".to_string(),
            start: 0.0,
            end: 0.5,
        };
        let word2 = CartesiaWord {
            word: "test".to_string(),
            start: 0.0,
            end: 0.5,
        };
        assert_eq!(word1, word2);
    }
}

// =============================================================================
// CartesiaTranscript Tests
// =============================================================================

mod transcript_tests {
    use super::*;
    use crate::core::stt::STTResult;

    #[test]
    fn test_transcript_to_stt_result() {
        let transcript = CartesiaTranscript {
            text: "Hello world".to_string(),
            is_final: true,
            words: None,
        };

        let result = transcript.to_stt_result();
        assert_eq!(result.transcript, "Hello world");
        assert!(result.is_final);
        assert!(result.is_speech_final);
        assert_eq!(result.confidence, 1.0);
    }

    #[test]
    fn test_transcript_from_trait() {
        let transcript = CartesiaTranscript {
            text: "Test".to_string(),
            is_final: true,
            words: None,
        };

        let result: STTResult = (&transcript).into();
        assert_eq!(result.transcript, "Test");
        assert!(result.is_final);
    }
}

// =============================================================================
// CartesiaSTT Client Tests
// =============================================================================

mod client_tests {
    use super::*;

    #[test]
    fn test_default() {
        let stt = CartesiaSTT::default();
        // Use public method to check config
        assert!(stt.get_config().is_none());
        assert!(!stt.is_ready());
    }

    #[tokio::test]
    async fn test_new_valid_config() {
        let config = STTConfig {
            api_key: "test_key".to_string(),
            language: "en".to_string(),
            sample_rate: 16000,
            ..Default::default()
        };

        let stt = <CartesiaSTT as BaseSTT>::new(config);
        assert!(stt.is_ok());

        let stt = stt.unwrap();
        assert!(!stt.is_ready());
        assert_eq!(stt.get_provider_info(), "Cartesia STT (ink-whisper)");
    }

    #[tokio::test]
    async fn test_new_empty_api_key() {
        let config = STTConfig {
            api_key: String::new(),
            ..Default::default()
        };

        let result = <CartesiaSTT as BaseSTT>::new(config);
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
            language: "fr".to_string(),
            sample_rate: 24000,
            ..Default::default()
        };

        let stt = <CartesiaSTT as BaseSTT>::new(config).unwrap();
        let retrieved_config = stt.get_config();

        assert!(retrieved_config.is_some());
        let retrieved_config = retrieved_config.unwrap();
        assert_eq!(retrieved_config.api_key, "test_key");
        assert_eq!(retrieved_config.language, "fr");
        assert_eq!(retrieved_config.sample_rate, 24000);
    }

    #[tokio::test]
    async fn test_is_ready_before_connect() {
        let config = STTConfig {
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let stt = <CartesiaSTT as BaseSTT>::new(config).unwrap();
        assert!(!stt.is_ready());
    }

    #[tokio::test]
    async fn test_send_audio_not_connected() {
        let config = STTConfig {
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let mut stt = <CartesiaSTT as BaseSTT>::new(config).unwrap();
        let result = stt.send_audio(vec![0u8; 100]).await;

        assert!(result.is_err());
        match result {
            Err(STTError::ConnectionFailed(_)) => {}
            _ => panic!("Expected ConnectionFailed error"),
        }
    }

    #[tokio::test]
    async fn test_callback_registration() {
        use std::future::Future;
        use std::pin::Pin;
        use std::sync::Arc;

        let config = STTConfig {
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let mut stt = <CartesiaSTT as BaseSTT>::new(config).unwrap();

        // Register result callback
        let result_callback = Arc::new(|_result: crate::core::stt::base::STTResult| {
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
    async fn test_disconnect_when_not_connected() {
        let config = STTConfig {
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let mut stt = <CartesiaSTT as BaseSTT>::new(config).unwrap();
        let result = stt.disconnect().await;
        assert!(result.is_ok());
        assert!(!stt.is_ready());
    }

    #[tokio::test]
    async fn test_provider_info() {
        let config = STTConfig {
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let stt = <CartesiaSTT as BaseSTT>::new(config).unwrap();
        assert_eq!(stt.get_provider_info(), "Cartesia STT (ink-whisper)");
    }
}

// =============================================================================
// Edge Case Tests
// =============================================================================

mod edge_case_tests {
    use super::*;

    // =========================================================================
    // Empty and Whitespace Transcript Tests
    // =========================================================================

    #[test]
    fn test_empty_transcript_text() {
        let json = r#"{"type": "transcript", "text": "", "is_final": true}"#;
        let msg = CartesiaMessage::parse(json).unwrap();

        let transcript = msg.as_transcript().unwrap();
        assert_eq!(transcript.text, "");
        assert!(transcript.is_final);
    }

    #[test]
    fn test_whitespace_only_transcript() {
        let json = r#"{"type": "transcript", "text": "   ", "is_final": false}"#;
        let msg = CartesiaMessage::parse(json).unwrap();

        let transcript = msg.as_transcript().unwrap();
        assert_eq!(transcript.text, "   ");
        assert!(!transcript.is_final);
    }

    #[test]
    fn test_transcript_with_newlines() {
        let json = r#"{"type": "transcript", "text": "Hello\nWorld", "is_final": true}"#;
        let msg = CartesiaMessage::parse(json).unwrap();

        let transcript = msg.as_transcript().unwrap();
        assert_eq!(transcript.text, "Hello\nWorld");
    }

    // =========================================================================
    // Unicode Tests
    // =========================================================================

    #[test]
    fn test_transcript_with_unicode() {
        let json = r#"{"type": "transcript", "text": "こんにちは世界", "is_final": true}"#;
        let msg = CartesiaMessage::parse(json).unwrap();

        let transcript = msg.as_transcript().unwrap();
        assert_eq!(transcript.text, "こんにちは世界");
    }

    #[test]
    fn test_transcript_with_emoji() {
        let json = r#"{"type": "transcript", "text": "Hello 👋 World 🌍", "is_final": true}"#;
        let msg = CartesiaMessage::parse(json).unwrap();

        let transcript = msg.as_transcript().unwrap();
        assert_eq!(transcript.text, "Hello 👋 World 🌍");
    }

    #[test]
    fn test_transcript_with_arabic() {
        let json = r#"{"type": "transcript", "text": "مرحبا بالعالم", "is_final": true}"#;
        let msg = CartesiaMessage::parse(json).unwrap();

        let transcript = msg.as_transcript().unwrap();
        assert_eq!(transcript.text, "مرحبا بالعالم");
    }

    #[test]
    fn test_transcript_with_mixed_scripts() {
        let json = r#"{"type": "transcript", "text": "Hello 你好 Привет", "is_final": true}"#;
        let msg = CartesiaMessage::parse(json).unwrap();

        let transcript = msg.as_transcript().unwrap();
        assert_eq!(transcript.text, "Hello 你好 Привет");
    }

    // =========================================================================
    // Words Array Edge Cases
    // =========================================================================

    #[test]
    fn test_transcript_with_empty_words_array() {
        let json = r#"{"type": "transcript", "text": "Hello", "is_final": true, "words": []}"#;
        let msg = CartesiaMessage::parse(json).unwrap();

        let transcript = msg.as_transcript().unwrap();
        assert!(transcript.words.is_some());
        assert!(transcript.words.as_ref().unwrap().is_empty());
    }

    #[test]
    fn test_transcript_words_missing_field() {
        // words is optional, should deserialize without it
        let json = r#"{"type": "transcript", "text": "Hello", "is_final": true}"#;
        let msg = CartesiaMessage::parse(json).unwrap();

        let transcript = msg.as_transcript().unwrap();
        assert!(transcript.words.is_none());
    }

    #[test]
    fn test_word_with_zero_duration() {
        let json = r#"{"word": "um", "start": 1.5, "end": 1.5}"#;
        let word: CartesiaWord = serde_json::from_str(json).unwrap();

        assert_eq!(word.word, "um");
        assert_eq!(word.start, 1.5);
        assert_eq!(word.end, 1.5);
    }

    #[test]
    fn test_word_with_fractional_timestamps() {
        let json = r#"{"word": "test", "start": 1.234567, "end": 2.345678}"#;
        let word: CartesiaWord = serde_json::from_str(json).unwrap();

        assert!((word.start - 1.234567).abs() < 0.0001);
        assert!((word.end - 2.345678).abs() < 0.0001);
    }

    #[test]
    fn test_word_with_very_long_duration() {
        let json = r#"{"word": "pause", "start": 0.0, "end": 3600.0}"#;
        let word: CartesiaWord = serde_json::from_str(json).unwrap();

        assert_eq!(word.end, 3600.0); // 1 hour
    }

    // =========================================================================
    // Error Message Tests
    // =========================================================================

    #[test]
    fn test_error_with_empty_message() {
        let json = r#"{"type": "error", "message": ""}"#;
        let msg = CartesiaMessage::parse(json).unwrap();

        assert!(msg.is_error());
        let error = msg.as_error().unwrap();
        assert_eq!(error.message, "");
    }

    #[test]
    fn test_error_with_long_message() {
        let long_message = "x".repeat(10000);
        let json = format!(r#"{{"type": "error", "message": "{}"}}"#, long_message);
        let msg = CartesiaMessage::parse(&json).unwrap();

        assert!(msg.is_error());
        let error = msg.as_error().unwrap();
        assert_eq!(error.message.len(), 10000);
    }

    #[test]
    fn test_error_with_special_characters() {
        let json = r#"{"type": "error", "message": "Error: \"Invalid\" <audio> & format"}"#;
        let msg = CartesiaMessage::parse(json).unwrap();

        assert!(msg.is_error());
        let error = msg.as_error().unwrap();
        assert!(error.message.contains("Invalid"));
        assert!(error.message.contains("<audio>"));
    }

    // =========================================================================
    // Invalid JSON Tests
    // =========================================================================

    #[test]
    fn test_parse_malformed_json() {
        let json = "{not valid json";
        let result = CartesiaMessage::parse(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_missing_type_field() {
        let json = r#"{"text": "Hello", "is_final": true}"#;
        let result = CartesiaMessage::parse(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_wrong_type_for_is_final() {
        let json = r#"{"type": "transcript", "text": "Hello", "is_final": "yes"}"#;
        let result = CartesiaMessage::parse(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_null_text() {
        let json = r#"{"type": "transcript", "text": null, "is_final": true}"#;
        let result = CartesiaMessage::parse(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_empty_object() {
        let json = "{}";
        let result = CartesiaMessage::parse(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_array_instead_of_object() {
        let json = "[]";
        let result = CartesiaMessage::parse(json);
        assert!(result.is_err());
    }

    // =========================================================================
    // Long/Large Content Tests
    // =========================================================================

    #[test]
    fn test_very_long_transcript() {
        let long_text = "word ".repeat(10000);
        let json = format!(
            r#"{{"type": "transcript", "text": "{}", "is_final": true}}"#,
            long_text.trim()
        );
        let msg = CartesiaMessage::parse(&json).unwrap();

        let transcript = msg.as_transcript().unwrap();
        assert!(transcript.text.len() > 40000);
        assert!(transcript.is_final);
    }

    #[test]
    fn test_many_words() {
        let words: Vec<String> = (0..1000)
            .map(|i| {
                format!(
                    r#"{{"word": "word{}", "start": {}.0, "end": {}.5}}"#,
                    i, i, i
                )
            })
            .collect();

        let json = format!(
            r#"{{"type": "transcript", "text": "many words", "is_final": true, "words": [{}]}}"#,
            words.join(",")
        );

        let msg = CartesiaMessage::parse(&json).unwrap();
        let transcript = msg.as_transcript().unwrap();
        assert_eq!(transcript.words.as_ref().unwrap().len(), 1000);
    }

    // =========================================================================
    // STTResult Conversion Edge Cases
    // =========================================================================

    #[test]
    fn test_empty_transcript_to_stt_result() {
        let transcript = CartesiaTranscript {
            text: String::new(),
            is_final: true,
            words: None,
        };

        let result = transcript.to_stt_result();
        assert_eq!(result.transcript, "");
        assert!(result.is_final);
        assert!(result.is_speech_final);
        assert_eq!(result.confidence, 1.0);
    }

    #[test]
    fn test_interim_transcript_has_zero_confidence() {
        let transcript = CartesiaTranscript {
            text: "partial".to_string(),
            is_final: false,
            words: None,
        };

        let result = transcript.to_stt_result();
        assert_eq!(result.confidence, 0.0);
    }

    #[test]
    fn test_final_transcript_has_full_confidence() {
        let transcript = CartesiaTranscript {
            text: "final".to_string(),
            is_final: true,
            words: None,
        };

        let result = transcript.to_stt_result();
        assert_eq!(result.confidence, 1.0);
    }
}

// =============================================================================
// Note: WebSocket Message Handling Tests
// =============================================================================
//
// WebSocket message handling tests are located in the client.rs module's
// internal test section. These tests cover:
// - handle_websocket_message for transcript, done, flush_done, error messages
// - Close message handling
// - Error forwarding via channels
//
// See src/core/stt/cartesia/client.rs tests module for these tests.
