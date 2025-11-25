use std::time::Duration;

use bytes::Bytes;
use google_api_proto::google::cloud::speech::v2::explicit_decoding_config::AudioEncoding;
use google_api_proto::google::cloud::speech::v2::streaming_recognize_response::SpeechEventType;
use google_api_proto::google::cloud::speech::v2::{
    SpeechRecognitionAlternative, StreamingRecognitionResult, StreamingRecognizeResponse,
    streaming_recognize_request::StreamingRequest,
};
use tokio::sync::mpsc;

use crate::core::stt::base::{STTConfig, STTError, STTResult};
use crate::core::stt::google::GoogleSTTConfig;
use crate::core::stt::google::streaming::{
    KEEPALIVE_INTERVAL_SECS, KEEPALIVE_SILENCE_DURATION_MS, KeepaliveTracker, MAX_AUDIO_CHUNK_SIZE,
    build_audio_request, build_config_request, chunk_audio, chunk_audio_vec,
    determine_speech_final, generate_silence_audio, get_confidence, handle_grpc_error,
    handle_streaming_response, map_encoding_to_proto, to_prost_duration,
};

#[test]
fn test_map_encoding_to_proto() {
    assert_eq!(
        map_encoding_to_proto("linear16") as i32,
        AudioEncoding::Linear16 as i32
    );
    assert_eq!(
        map_encoding_to_proto("LINEAR16") as i32,
        AudioEncoding::Linear16 as i32
    );
    assert_eq!(
        map_encoding_to_proto("pcm") as i32,
        AudioEncoding::Linear16 as i32
    );
    assert_eq!(
        map_encoding_to_proto("mulaw") as i32,
        AudioEncoding::Mulaw as i32
    );
    assert_eq!(
        map_encoding_to_proto("ulaw") as i32,
        AudioEncoding::Mulaw as i32
    );
    assert_eq!(
        map_encoding_to_proto("alaw") as i32,
        AudioEncoding::Alaw as i32
    );
    assert_eq!(
        map_encoding_to_proto("unknown") as i32,
        AudioEncoding::Linear16 as i32
    );
}

#[test]
fn test_to_prost_duration() {
    let std_duration = Duration::from_secs(5);
    let prost_duration = to_prost_duration(std_duration);
    assert_eq!(prost_duration.seconds, 5);
    assert_eq!(prost_duration.nanos, 0);

    let std_duration = Duration::from_millis(1500);
    let prost_duration = to_prost_duration(std_duration);
    assert_eq!(prost_duration.seconds, 1);
    assert_eq!(prost_duration.nanos, 500_000_000);

    let std_duration = Duration::from_nanos(123_456_789);
    let prost_duration = to_prost_duration(std_duration);
    assert_eq!(prost_duration.seconds, 0);
    assert_eq!(prost_duration.nanos, 123_456_789);
}

#[test]
fn test_chunk_audio_small() {
    let audio = vec![0u8; 1000];
    let chunks = chunk_audio_vec(audio.clone());
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0], audio);
}

#[test]
fn test_chunk_audio_exact_max() {
    let audio = vec![0u8; MAX_AUDIO_CHUNK_SIZE];
    let chunks = chunk_audio_vec(audio.clone());
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].len(), MAX_AUDIO_CHUNK_SIZE);
}

#[test]
fn test_chunk_audio_large() {
    let size = MAX_AUDIO_CHUNK_SIZE * 2 + 1000;
    let audio: Vec<u8> = (0..size).map(|i| (i % 256) as u8).collect();
    let chunks = chunk_audio_vec(audio);

    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0].len(), MAX_AUDIO_CHUNK_SIZE);
    assert_eq!(chunks[1].len(), MAX_AUDIO_CHUNK_SIZE);
    assert_eq!(chunks[2].len(), 1000);
}

#[test]
fn test_chunk_audio_iterator() {
    // Test the new iterator-based API directly
    let audio = Bytes::from(vec![0u8; MAX_AUDIO_CHUNK_SIZE + 100]);
    let chunks: Vec<_> = chunk_audio(audio).collect();
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].len(), MAX_AUDIO_CHUNK_SIZE);
    assert_eq!(chunks[1].len(), 100);
}

#[test]
fn test_build_config_request() {
    let config = GoogleSTTConfig {
        base: STTConfig {
            provider: "google".to_string(),
            api_key: String::new(),
            language: "en-US".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            model: "latest_long".to_string(),
        },
        project_id: "test-project".to_string(),
        location: "global".to_string(),
        recognizer_id: None,
        interim_results: true,
        enable_voice_activity_events: true,
        speech_start_timeout: None,
        speech_end_timeout: None,
        single_utterance: false,
    };

    let request = build_config_request(&config);

    assert_eq!(
        request.recognizer,
        "projects/test-project/locations/global/recognizers/_"
    );

    match request.streaming_request {
        Some(StreamingRequest::StreamingConfig(streaming_config)) => {
            let rec_config = streaming_config.config.unwrap();
            assert_eq!(rec_config.language_codes, vec!["en-US"]);
            assert_eq!(rec_config.model, "latest_long");

            let features = rec_config.features.unwrap();
            assert!(features.enable_automatic_punctuation);

            let streaming_features = streaming_config.streaming_features.unwrap();
            assert!(streaming_features.interim_results);
            assert!(streaming_features.enable_voice_activity_events);
        }
        _ => panic!("Expected StreamingConfig variant"),
    }
}

#[test]
fn test_build_config_request_with_timeouts() {
    let config = GoogleSTTConfig {
        base: STTConfig::default(),
        project_id: "test-project".to_string(),
        location: "global".to_string(),
        recognizer_id: None,
        interim_results: true,
        enable_voice_activity_events: true,
        speech_start_timeout: Some(Duration::from_secs(5)),
        speech_end_timeout: Some(Duration::from_millis(1500)),
        single_utterance: false,
    };

    let request = build_config_request(&config);

    match request.streaming_request {
        Some(StreamingRequest::StreamingConfig(streaming_config)) => {
            let streaming_features = streaming_config.streaming_features.unwrap();
            let timeout = streaming_features.voice_activity_timeout.unwrap();

            let start_timeout = timeout.speech_start_timeout.unwrap();
            assert_eq!(start_timeout.seconds, 5);
            assert_eq!(start_timeout.nanos, 0);

            let end_timeout = timeout.speech_end_timeout.unwrap();
            assert_eq!(end_timeout.seconds, 1);
            assert_eq!(end_timeout.nanos, 500_000_000);
        }
        _ => panic!("Expected StreamingConfig variant"),
    }
}

#[test]
fn test_build_audio_request() {
    let audio_data = vec![1u8, 2, 3, 4, 5];
    let recognizer = "projects/test/locations/global/recognizers/_".to_string();

    let request = build_audio_request(Bytes::from(audio_data.clone()), recognizer.clone());

    assert_eq!(request.recognizer, recognizer);

    match request.streaming_request {
        Some(StreamingRequest::Audio(bytes)) => {
            assert_eq!(bytes.as_ref(), audio_data.as_slice());
        }
        _ => panic!("Expected Audio variant"),
    }
}

#[test]
fn test_handle_streaming_response_with_results() {
    let (tx, mut rx) = mpsc::unbounded_channel::<STTResult>();

    let response = StreamingRecognizeResponse {
        results: vec![StreamingRecognitionResult {
            alternatives: vec![SpeechRecognitionAlternative {
                transcript: "Hello world".to_string(),
                confidence: 0.95,
                words: vec![],
            }],
            is_final: true,
            stability: 0.0,
            result_end_offset: None,
            channel_tag: 0,
            language_code: "en-US".to_string(),
        }],
        speech_event_type: 0,
        speech_event_offset: None,
        metadata: None,
    };

    handle_streaming_response(response, &tx).unwrap();

    let result = rx.try_recv().unwrap();
    assert_eq!(result.transcript, "Hello world");
    assert!(result.is_final);
    assert!(result.is_speech_final);
    assert_eq!(result.confidence, 0.95);
}

#[test]
fn test_handle_streaming_response_empty_results() {
    let (tx, mut rx) = mpsc::unbounded_channel::<STTResult>();

    let response = StreamingRecognizeResponse {
        results: vec![],
        speech_event_type: 0,
        speech_event_offset: None,
        metadata: None,
    };

    handle_streaming_response(response, &tx).unwrap();

    assert!(rx.try_recv().is_err());
}

#[test]
fn test_handle_streaming_response_empty_transcript() {
    let (tx, mut rx) = mpsc::unbounded_channel::<STTResult>();

    let response = StreamingRecognizeResponse {
        results: vec![StreamingRecognitionResult {
            alternatives: vec![SpeechRecognitionAlternative {
                transcript: String::new(),
                confidence: 0.0,
                words: vec![],
            }],
            is_final: false,
            stability: 0.0,
            result_end_offset: None,
            channel_tag: 0,
            language_code: String::new(),
        }],
        speech_event_type: 0,
        speech_event_offset: None,
        metadata: None,
    };

    handle_streaming_response(response, &tx).unwrap();

    assert!(rx.try_recv().is_err());
}

#[test]
fn test_handle_streaming_response_speech_events() {
    let (tx, mut rx) = mpsc::unbounded_channel::<STTResult>();

    let response = StreamingRecognizeResponse {
        results: vec![],
        speech_event_type: 2,
        speech_event_offset: None,
        metadata: None,
    };
    handle_streaming_response(response, &tx).unwrap();
    assert!(rx.try_recv().is_err());

    let response = StreamingRecognizeResponse {
        results: vec![],
        speech_event_type: 3,
        speech_event_offset: None,
        metadata: None,
    };
    handle_streaming_response(response, &tx).unwrap();
    assert!(rx.try_recv().is_err());

    let response = StreamingRecognizeResponse {
        results: vec![],
        speech_event_type: 1,
        speech_event_offset: None,
        metadata: None,
    };
    handle_streaming_response(response, &tx).unwrap();
    assert!(rx.try_recv().is_err());
}

#[test]
fn test_max_audio_chunk_size_constant() {
    assert_eq!(MAX_AUDIO_CHUNK_SIZE, 25 * 1024);
}

#[test]
fn test_determine_speech_final_end_of_utterance() {
    assert!(determine_speech_final(
        SpeechEventType::EndOfSingleUtterance,
        true
    ));
    assert!(determine_speech_final(
        SpeechEventType::EndOfSingleUtterance,
        false
    ));
}

#[test]
fn test_determine_speech_final_activity_end() {
    assert!(determine_speech_final(
        SpeechEventType::SpeechActivityEnd,
        true
    ));
    assert!(!determine_speech_final(
        SpeechEventType::SpeechActivityEnd,
        false
    ));
}

#[test]
fn test_determine_speech_final_activity_begin() {
    assert!(determine_speech_final(
        SpeechEventType::SpeechActivityBegin,
        true
    ));
    assert!(!determine_speech_final(
        SpeechEventType::SpeechActivityBegin,
        false
    ));
}

#[test]
fn test_determine_speech_final_unspecified() {
    assert!(determine_speech_final(SpeechEventType::Unspecified, true));
    assert!(!determine_speech_final(SpeechEventType::Unspecified, false));
}

#[test]
fn test_get_confidence_final_with_confidence() {
    assert_eq!(get_confidence(0.95, true), 0.95);
    assert_eq!(get_confidence(0.5, true), 0.5);
    assert_eq!(get_confidence(1.0, true), 1.0);
}

#[test]
fn test_get_confidence_final_zero_confidence() {
    assert_eq!(get_confidence(0.0, true), 0.0);
}

#[test]
fn test_get_confidence_interim() {
    assert_eq!(get_confidence(0.95, false), 0.0);
    assert_eq!(get_confidence(0.5, false), 0.0);
    assert_eq!(get_confidence(0.0, false), 0.0);
}

#[test]
fn test_get_confidence_negative() {
    assert_eq!(get_confidence(-0.5, true), 0.0);
    assert_eq!(get_confidence(-0.5, false), 0.0);
}

#[test]
fn test_handle_grpc_error_unauthenticated() {
    let status = tonic::Status::unauthenticated("Invalid credentials");
    let error = handle_grpc_error(status);
    match error {
        STTError::AuthenticationFailed(msg) => {
            assert_eq!(msg, "Invalid credentials");
        }
        _ => panic!("Expected AuthenticationFailed error"),
    }
}

#[test]
fn test_handle_grpc_error_permission_denied() {
    let status = tonic::Status::permission_denied("Access denied");
    let error = handle_grpc_error(status);
    match error {
        STTError::AuthenticationFailed(msg) => {
            assert!(msg.contains("Permission denied"));
            assert!(msg.contains("Access denied"));
        }
        _ => panic!("Expected AuthenticationFailed error"),
    }
}

#[test]
fn test_handle_grpc_error_invalid_argument() {
    let status = tonic::Status::invalid_argument("Bad request");
    let error = handle_grpc_error(status);
    match error {
        STTError::ConfigurationError(msg) => {
            assert_eq!(msg, "Bad request");
        }
        _ => panic!("Expected ConfigurationError"),
    }
}

#[test]
fn test_handle_grpc_error_resource_exhausted() {
    let status = tonic::Status::resource_exhausted("Quota exceeded");
    let error = handle_grpc_error(status);
    match error {
        STTError::ProviderError(msg) => {
            assert!(msg.contains("Rate limit exceeded"));
            assert!(msg.contains("Quota exceeded"));
        }
        _ => panic!("Expected ProviderError"),
    }
}

#[test]
fn test_handle_grpc_error_unavailable() {
    let status = tonic::Status::unavailable("Service down");
    let error = handle_grpc_error(status);
    match error {
        STTError::NetworkError(msg) => {
            assert!(msg.contains("Service unavailable"));
            assert!(msg.contains("Service down"));
        }
        _ => panic!("Expected NetworkError"),
    }
}

#[test]
fn test_handle_grpc_error_deadline_exceeded() {
    let status = tonic::Status::deadline_exceeded("Timed out");
    let error = handle_grpc_error(status);
    match error {
        STTError::NetworkError(msg) => {
            assert!(msg.contains("Request timeout"));
            assert!(msg.contains("Timed out"));
        }
        _ => panic!("Expected NetworkError"),
    }
}

#[test]
fn test_handle_grpc_error_cancelled() {
    let status = tonic::Status::cancelled("User cancelled");
    let error = handle_grpc_error(status);
    match error {
        STTError::ProviderError(msg) => {
            assert!(msg.contains("Request cancelled"));
            assert!(msg.contains("User cancelled"));
        }
        _ => panic!("Expected ProviderError"),
    }
}

#[test]
fn test_handle_grpc_error_internal() {
    let status = tonic::Status::internal("Internal error");
    let error = handle_grpc_error(status);
    match error {
        STTError::ProviderError(msg) => {
            assert!(msg.contains("Internal error"));
        }
        _ => panic!("Expected ProviderError"),
    }
}

#[test]
fn test_handle_grpc_error_unknown() {
    let status = tonic::Status::unknown("Unknown error");
    let error = handle_grpc_error(status);
    match error {
        STTError::ProviderError(msg) => {
            assert!(msg.contains("gRPC error"));
            assert!(msg.contains("Unknown error"));
        }
        _ => panic!("Expected ProviderError"),
    }
}

#[test]
fn test_handle_streaming_response_interim_result() {
    let (tx, mut rx) = mpsc::unbounded_channel::<STTResult>();

    let response = StreamingRecognizeResponse {
        results: vec![StreamingRecognitionResult {
            alternatives: vec![SpeechRecognitionAlternative {
                transcript: "Hello".to_string(),
                confidence: 0.0,
                words: vec![],
            }],
            is_final: false,
            stability: 0.8,
            result_end_offset: None,
            channel_tag: 0,
            language_code: "en-US".to_string(),
        }],
        speech_event_type: 0,
        speech_event_offset: None,
        metadata: None,
    };

    handle_streaming_response(response, &tx).unwrap();

    let result = rx.try_recv().unwrap();
    assert_eq!(result.transcript, "Hello");
    assert!(!result.is_final);
    assert!(!result.is_speech_final);
    assert_eq!(result.confidence, 0.0);
}

#[test]
fn test_handle_streaming_response_with_end_of_utterance() {
    let (tx, mut rx) = mpsc::unbounded_channel::<STTResult>();

    let response = StreamingRecognizeResponse {
        results: vec![StreamingRecognitionResult {
            alternatives: vec![SpeechRecognitionAlternative {
                transcript: "Hello world".to_string(),
                confidence: 0.95,
                words: vec![],
            }],
            is_final: true,
            stability: 0.0,
            result_end_offset: None,
            channel_tag: 0,
            language_code: "en-US".to_string(),
        }],
        speech_event_type: 1,
        speech_event_offset: None,
        metadata: None,
    };

    handle_streaming_response(response, &tx).unwrap();

    let result = rx.try_recv().unwrap();
    assert_eq!(result.transcript, "Hello world");
    assert!(result.is_final);
    assert!(result.is_speech_final);
    assert_eq!(result.confidence, 0.95);
}

#[test]
fn test_handle_streaming_response_speech_activity_end_with_final() {
    let (tx, mut rx) = mpsc::unbounded_channel::<STTResult>();

    let response = StreamingRecognizeResponse {
        results: vec![StreamingRecognitionResult {
            alternatives: vec![SpeechRecognitionAlternative {
                transcript: "Hello".to_string(),
                confidence: 0.9,
                words: vec![],
            }],
            is_final: true,
            stability: 0.0,
            result_end_offset: None,
            channel_tag: 0,
            language_code: "en-US".to_string(),
        }],
        speech_event_type: 3,
        speech_event_offset: None,
        metadata: None,
    };

    handle_streaming_response(response, &tx).unwrap();

    let result = rx.try_recv().unwrap();
    assert!(result.is_final);
    assert!(result.is_speech_final);
}

#[test]
fn test_handle_streaming_response_multiple_results() {
    let (tx, mut rx) = mpsc::unbounded_channel::<STTResult>();

    let response = StreamingRecognizeResponse {
        results: vec![
            StreamingRecognitionResult {
                alternatives: vec![SpeechRecognitionAlternative {
                    transcript: "First".to_string(),
                    confidence: 0.9,
                    words: vec![],
                }],
                is_final: true,
                stability: 0.0,
                result_end_offset: None,
                channel_tag: 0,
                language_code: "en-US".to_string(),
            },
            StreamingRecognitionResult {
                alternatives: vec![SpeechRecognitionAlternative {
                    transcript: "Second".to_string(),
                    confidence: 0.85,
                    words: vec![],
                }],
                is_final: true,
                stability: 0.0,
                result_end_offset: None,
                channel_tag: 0,
                language_code: "en-US".to_string(),
            },
        ],
        speech_event_type: 0,
        speech_event_offset: None,
        metadata: None,
    };

    handle_streaming_response(response, &tx).unwrap();

    let result1 = rx.try_recv().unwrap();
    assert_eq!(result1.transcript, "First");
    assert_eq!(result1.confidence, 0.9);

    let result2 = rx.try_recv().unwrap();
    assert_eq!(result2.transcript, "Second");
    assert_eq!(result2.confidence, 0.85);
}

#[test]
fn test_handle_streaming_response_no_alternatives() {
    let (tx, mut rx) = mpsc::unbounded_channel::<STTResult>();

    let response = StreamingRecognizeResponse {
        results: vec![StreamingRecognitionResult {
            alternatives: vec![],
            is_final: true,
            stability: 0.0,
            result_end_offset: None,
            channel_tag: 0,
            language_code: "en-US".to_string(),
        }],
        speech_event_type: 0,
        speech_event_offset: None,
        metadata: None,
    };

    handle_streaming_response(response, &tx).unwrap();

    assert!(rx.try_recv().is_err());
}

#[test]
fn test_handle_streaming_response_channel_closed() {
    let (tx, rx) = mpsc::unbounded_channel::<STTResult>();
    drop(rx);

    let response = StreamingRecognizeResponse {
        results: vec![StreamingRecognitionResult {
            alternatives: vec![SpeechRecognitionAlternative {
                transcript: "Hello".to_string(),
                confidence: 0.95,
                words: vec![],
            }],
            is_final: true,
            stability: 0.0,
            result_end_offset: None,
            channel_tag: 0,
            language_code: "en-US".to_string(),
        }],
        speech_event_type: 0,
        speech_event_offset: None,
        metadata: None,
    };

    let result = handle_streaming_response(response, &tx);
    assert!(result.is_err());
    match result {
        Err(STTError::ProviderError(msg)) => {
            assert!(msg.contains("channel closed"));
        }
        _ => panic!("Expected ProviderError with channel closed message"),
    }
}

#[test]
fn test_handle_streaming_response_multiple_alternatives() {
    let (tx, mut rx) = mpsc::unbounded_channel::<STTResult>();

    let response = StreamingRecognizeResponse {
        results: vec![StreamingRecognitionResult {
            alternatives: vec![
                SpeechRecognitionAlternative {
                    transcript: "Best transcription".to_string(),
                    confidence: 0.95,
                    words: vec![],
                },
                SpeechRecognitionAlternative {
                    transcript: "Second best".to_string(),
                    confidence: 0.80,
                    words: vec![],
                },
            ],
            is_final: true,
            stability: 0.0,
            result_end_offset: None,
            channel_tag: 0,
            language_code: "en-US".to_string(),
        }],
        speech_event_type: 0,
        speech_event_offset: None,
        metadata: None,
    };

    handle_streaming_response(response, &tx).unwrap();

    let result = rx.try_recv().unwrap();
    assert_eq!(result.transcript, "Best transcription");
    assert_eq!(result.confidence, 0.95);
}

#[test]
fn test_chunk_audio_empty() {
    // Empty audio should return zero chunks (nothing to process)
    // This is the correct behavior for streaming - don't send empty packets
    let audio: Vec<u8> = vec![];
    let chunks = chunk_audio_vec(audio);
    assert_eq!(chunks.len(), 0);
}

#[test]
fn test_chunk_audio_one_byte_over_limit() {
    let size = MAX_AUDIO_CHUNK_SIZE + 1;
    let audio = vec![0u8; size];
    let chunks = chunk_audio_vec(audio);

    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].len(), MAX_AUDIO_CHUNK_SIZE);
    assert_eq!(chunks[1].len(), 1);
}

#[test]
fn test_chunk_audio_preserves_data_integrity() {
    let size = MAX_AUDIO_CHUNK_SIZE * 2 + 100;
    let audio: Vec<u8> = (0..size).map(|i| (i % 256) as u8).collect();
    let chunks = chunk_audio_vec(audio.clone());

    let reconstructed: Vec<u8> = chunks.into_iter().flatten().collect();
    assert_eq!(reconstructed, audio);
}

#[test]
fn test_build_config_request_without_vad_timeouts() {
    let config = GoogleSTTConfig {
        project_id: "test-project".to_string(),
        speech_start_timeout: None,
        speech_end_timeout: None,
        ..Default::default()
    };

    let request = build_config_request(&config);

    match request.streaming_request {
        Some(StreamingRequest::StreamingConfig(streaming_config)) => {
            let streaming_features = streaming_config.streaming_features.unwrap();
            assert!(streaming_features.voice_activity_timeout.is_none());
        }
        _ => panic!("Expected StreamingConfig variant"),
    }
}

#[test]
fn test_build_config_request_with_only_start_timeout() {
    let config = GoogleSTTConfig {
        project_id: "test-project".to_string(),
        speech_start_timeout: Some(Duration::from_secs(3)),
        speech_end_timeout: None,
        ..Default::default()
    };

    let request = build_config_request(&config);

    match request.streaming_request {
        Some(StreamingRequest::StreamingConfig(streaming_config)) => {
            let streaming_features = streaming_config.streaming_features.unwrap();
            let timeout = streaming_features.voice_activity_timeout.unwrap();
            assert!(timeout.speech_start_timeout.is_some());
            assert!(timeout.speech_end_timeout.is_none());
        }
        _ => panic!("Expected StreamingConfig variant"),
    }
}

#[test]
fn test_build_config_request_with_only_end_timeout() {
    let config = GoogleSTTConfig {
        project_id: "test-project".to_string(),
        speech_start_timeout: None,
        speech_end_timeout: Some(Duration::from_millis(800)),
        ..Default::default()
    };

    let request = build_config_request(&config);

    match request.streaming_request {
        Some(StreamingRequest::StreamingConfig(streaming_config)) => {
            let streaming_features = streaming_config.streaming_features.unwrap();
            let timeout = streaming_features.voice_activity_timeout.unwrap();
            assert!(timeout.speech_start_timeout.is_none());
            assert!(timeout.speech_end_timeout.is_some());

            let end_timeout = timeout.speech_end_timeout.unwrap();
            assert_eq!(end_timeout.seconds, 0);
            assert_eq!(end_timeout.nanos, 800_000_000);
        }
        _ => panic!("Expected StreamingConfig variant"),
    }
}

#[test]
fn test_build_config_request_with_disabled_features() {
    let config = GoogleSTTConfig {
        base: STTConfig {
            punctuation: false,
            ..STTConfig::default()
        },
        project_id: "test-project".to_string(),
        interim_results: false,
        enable_voice_activity_events: false,
        ..Default::default()
    };

    let request = build_config_request(&config);

    match request.streaming_request {
        Some(StreamingRequest::StreamingConfig(streaming_config)) => {
            let rec_config = streaming_config.config.unwrap();
            let features = rec_config.features.unwrap();
            assert!(!features.enable_automatic_punctuation);

            let streaming_features = streaming_config.streaming_features.unwrap();
            assert!(!streaming_features.interim_results);
            assert!(!streaming_features.enable_voice_activity_events);
        }
        _ => panic!("Expected StreamingConfig variant"),
    }
}

#[test]
fn test_build_config_request_decoding_config() {
    use google_api_proto::google::cloud::speech::v2::recognition_config::DecodingConfig;

    let config = GoogleSTTConfig {
        base: STTConfig {
            encoding: "mulaw".to_string(),
            sample_rate: 8000,
            channels: 1,
            ..STTConfig::default()
        },
        project_id: "test-project".to_string(),
        ..Default::default()
    };

    let request = build_config_request(&config);

    match request.streaming_request {
        Some(StreamingRequest::StreamingConfig(streaming_config)) => {
            let rec_config = streaming_config.config.unwrap();
            match rec_config.decoding_config {
                Some(DecodingConfig::ExplicitDecodingConfig(explicit)) => {
                    assert_eq!(explicit.encoding, AudioEncoding::Mulaw as i32);
                    assert_eq!(explicit.sample_rate_hertz, 8000);
                    assert_eq!(explicit.audio_channel_count, 1);
                }
                _ => panic!("Expected ExplicitDecodingConfig"),
            }
        }
        _ => panic!("Expected StreamingConfig variant"),
    }
}

#[test]
fn test_stt_result_confidence_boundary_values() {
    let result = STTResult::new("test".to_string(), true, true, 0.0);
    assert_eq!(result.confidence, 0.0);

    let result = STTResult::new("test".to_string(), true, true, 1.0);
    assert_eq!(result.confidence, 1.0);

    let result = STTResult::new("test".to_string(), true, true, 0.5);
    assert_eq!(result.confidence, 0.5);
}

// ======== Keepalive functionality tests ========

#[test]
fn test_keepalive_constants() {
    // Verify the keepalive interval is reasonable (1 second, less than Google's ~10s timeout)
    assert_eq!(KEEPALIVE_INTERVAL_SECS, 1);

    // Verify silence duration is short enough not to affect speech detection
    assert_eq!(KEEPALIVE_SILENCE_DURATION_MS, 20);
}

#[test]
fn test_generate_silence_audio_16khz_mono() {
    // 16kHz, mono, 20ms = 16000 * 0.020 * 1 * 2 = 640 bytes
    let silence = generate_silence_audio(16000, 1, 20);
    assert_eq!(silence.len(), 640);

    // Verify all bytes are zero (silence for LINEAR16)
    assert!(silence.iter().all(|&b| b == 0));
}

#[test]
fn test_generate_silence_audio_8khz_mono() {
    // 8kHz, mono, 20ms = 8000 * 0.020 * 1 * 2 = 320 bytes
    let silence = generate_silence_audio(8000, 1, 20);
    assert_eq!(silence.len(), 320);
}

#[test]
fn test_generate_silence_audio_16khz_stereo() {
    // 16kHz, stereo, 20ms = 16000 * 0.020 * 2 * 2 = 1280 bytes
    let silence = generate_silence_audio(16000, 2, 20);
    assert_eq!(silence.len(), 1280);
}

#[test]
fn test_generate_silence_audio_48khz_mono() {
    // 48kHz, mono, 20ms = 48000 * 0.020 * 1 * 2 = 1920 bytes
    let silence = generate_silence_audio(48000, 1, 20);
    assert_eq!(silence.len(), 1920);
}

#[test]
fn test_generate_silence_audio_longer_duration() {
    // 16kHz, mono, 100ms = 16000 * 0.100 * 1 * 2 = 3200 bytes
    let silence = generate_silence_audio(16000, 1, 100);
    assert_eq!(silence.len(), 3200);
}

#[test]
fn test_keepalive_tracker_new() {
    let tracker = KeepaliveTracker::new(16000, 1);
    assert_eq!(tracker.sample_rate, 16000);
    assert_eq!(tracker.channels, 1);
}

#[test]
fn test_keepalive_tracker_needs_keepalive_initially_false() {
    let tracker = KeepaliveTracker::new(16000, 1);
    // Immediately after creation, should not need keepalive
    assert!(!tracker.needs_keepalive());
}

#[test]
fn test_keepalive_tracker_touch_resets_timer() {
    let mut tracker = KeepaliveTracker::new(16000, 1);

    // Touch should reset the last_activity
    let before = tracker.last_activity;
    std::thread::sleep(std::time::Duration::from_millis(10));
    tracker.touch();
    let after = tracker.last_activity;

    assert!(after > before);
}

#[test]
fn test_keepalive_tracker_generate_keepalive() {
    let tracker = KeepaliveTracker::new(16000, 1);
    let silence = tracker.generate_keepalive();

    // Should generate silence for the configured sample rate and duration
    // 16kHz, mono, 20ms = 640 bytes
    assert_eq!(silence.len(), 640);
    assert!(silence.iter().all(|&b| b == 0));
}

#[test]
fn test_keepalive_tracker_generate_keepalive_stereo() {
    let tracker = KeepaliveTracker::new(16000, 2);
    let silence = tracker.generate_keepalive();

    // 16kHz, stereo, 20ms = 1280 bytes
    assert_eq!(silence.len(), 1280);
}

#[tokio::test]
async fn test_keepalive_tracker_needs_keepalive_after_interval() {
    let tracker = KeepaliveTracker::new(16000, 1);

    // Initially should not need keepalive
    assert!(!tracker.needs_keepalive());

    // After waiting longer than KEEPALIVE_INTERVAL_SECS, should need keepalive
    tokio::time::sleep(tokio::time::Duration::from_secs(
        KEEPALIVE_INTERVAL_SECS + 1,
    ))
    .await;
    assert!(tracker.needs_keepalive());
}
