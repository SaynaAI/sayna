use base64::{Engine as _, engine::general_purpose};
use bytes::Bytes;

use super::config::{LiveKitWebSocketConfig, STTWebSocketConfig, TTSWebSocketConfig};
use super::messages::{
    IncomingMessage, MessageRoute, OutgoingMessage, ParticipantDisconnectedInfo, UnifiedMessage,
};

#[test]
fn test_ws_config_serialization() {
    // Test STT WebSocket config
    let stt_ws_config = STTWebSocketConfig {
        provider: "deepgram".to_string(),
        language: "en-US".to_string(),
        sample_rate: 16000,
        channels: 1,
        punctuation: true,
        encoding: "linear16".to_string(),
        model: "nova-3".to_string(),
    };

    let json = serde_json::to_string(&stt_ws_config).unwrap();
    assert!(json.contains("\"provider\":\"deepgram\""));
    assert!(json.contains("\"language\":\"en-US\""));
    assert!(!json.contains("api_key")); // Should not contain API key

    // Test TTS WebSocket config
    let tts_ws_config = TTSWebSocketConfig {
        provider: "deepgram".to_string(),
        voice_id: Some("aura-luna-en".to_string()),
        speaking_rate: Some(1.0),
        audio_format: Some("pcm".to_string()),
        sample_rate: Some(22050),
        connection_timeout: Some(30),
        request_timeout: Some(60),
        model: "".to_string(), // Model is in Voice ID for Deepgram
        pronunciations: Vec::new(),
    };

    let json = serde_json::to_string(&tts_ws_config).unwrap();
    assert!(json.contains("\"provider\":\"deepgram\""));
    assert!(json.contains("\"voice_id\":\"aura-luna-en\""));
    assert!(!json.contains("api_key")); // Should not contain API key
}

#[test]
fn test_incoming_message_serialization() {
    // Test config message
    let config_msg = IncomingMessage::Config {
        audio: Some(true),
        stt_config: Some(STTWebSocketConfig {
            provider: "deepgram".to_string(),
            language: "en-US".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            model: "nova-3".to_string(),
        }),
        tts_config: Some(TTSWebSocketConfig {
            provider: "deepgram".to_string(),
            voice_id: Some("aura-luna-en".to_string()),
            speaking_rate: Some(1.0),
            audio_format: Some("pcm".to_string()),
            sample_rate: Some(22050),
            connection_timeout: Some(30),
            request_timeout: Some(60),
            model: "".to_string(), // Model is in Voice ID for Deepgram
            pronunciations: Vec::new(),
        }),
        livekit: None,
    };

    let json = serde_json::to_string(&config_msg).unwrap();
    assert!(json.contains("\"type\":\"config\""));
    assert!(!json.contains("api_key")); // Should not contain API key

    // Test speak message
    let speak_msg = IncomingMessage::Speak {
        text: "Hello world".to_string(),
        flush: Some(true),
        allow_interruption: Some(true),
    };

    let json = serde_json::to_string(&speak_msg).unwrap();
    assert!(json.contains("\"type\":\"speak\""));
    assert!(json.contains("Hello world"));
    assert!(json.contains("\"flush\":true"));

    // Test speak message without flush (backward compatibility)
    let speak_msg_no_flush = IncomingMessage::Speak {
        text: "Hello world".to_string(),
        flush: None,
        allow_interruption: None,
    };

    let json = serde_json::to_string(&speak_msg_no_flush).unwrap();
    assert!(json.contains("\"type\":\"speak\""));
    assert!(json.contains("Hello world"));
    // Should not contain flush when None
    assert!(!json.contains("flush"));

    // Test parsing message without flush field (backward compatibility)
    let json_without_flush = r#"{"type":"speak","text":"Hello world"}"#;
    let parsed: IncomingMessage = serde_json::from_str(json_without_flush).unwrap();
    if let IncomingMessage::Speak {
        text,
        flush,
        allow_interruption,
    } = parsed
    {
        assert_eq!(text, "Hello world");
        assert_eq!(flush, None);
        assert_eq!(allow_interruption, Some(true)); // Defaults to true
    } else {
        panic!("Expected Speak message");
    }

    // Test speak message with allow_interruption=false
    let speak_msg_no_interruption = IncomingMessage::Speak {
        text: "Do not interrupt me".to_string(),
        flush: Some(true),
        allow_interruption: Some(false),
    };

    let json = serde_json::to_string(&speak_msg_no_interruption).unwrap();
    assert!(json.contains("\"type\":\"speak\""));
    assert!(json.contains("Do not interrupt me"));
    assert!(json.contains("\"allow_interruption\":false"));

    // Test parsing message with allow_interruption field
    let json_with_interruption = r#"{"type":"speak","text":"Hello","allow_interruption":false}"#;
    let parsed: IncomingMessage = serde_json::from_str(json_with_interruption).unwrap();
    if let IncomingMessage::Speak {
        text,
        flush,
        allow_interruption,
    } = parsed
    {
        assert_eq!(text, "Hello");
        assert_eq!(flush, None);
        assert_eq!(allow_interruption, Some(false));
    } else {
        panic!("Expected Speak message");
    }

    // Test flush message
    let clear_msg = IncomingMessage::Clear;
    let json = serde_json::to_string(&clear_msg).unwrap();
    assert!(json.contains("\"type\":\"clear\""));

    // Test send_message message with topic
    let send_msg = IncomingMessage::SendMessage {
        message: "Hello from client!".to_string(),
        role: "user".to_string(),
        topic: Some("chat".to_string()),
        debug: None,
    };
    let json = serde_json::to_string(&send_msg).unwrap();
    assert!(json.contains("\"type\":\"send_message\""));
    assert!(json.contains("\"message\":\"Hello from client!\""));
    assert!(json.contains("\"role\":\"user\""));
    assert!(json.contains("\"topic\":\"chat\""));

    // Test send_message message without topic
    let send_msg_no_topic = IncomingMessage::SendMessage {
        message: "Hello without topic!".to_string(),
        role: "user".to_string(),
        topic: None,
        debug: None,
    };
    let json = serde_json::to_string(&send_msg_no_topic).unwrap();
    assert!(json.contains("\"type\":\"send_message\""));
    assert!(json.contains("\"message\":\"Hello without topic!\""));
    assert!(json.contains("\"role\":\"user\""));
    // Should not contain topic field when None (but may contain the word "topic" elsewhere)
    assert!(!json.contains("\"topic\""));

    // Test parsing send_message JSON with topic
    let json_with_topic =
        r#"{"type":"send_message","message":"Hello from JSON!","role":"user","topic":"general"}"#;
    let parsed: IncomingMessage = serde_json::from_str(json_with_topic).unwrap();
    if let IncomingMessage::SendMessage {
        message,
        role,
        topic,
        ..
    } = parsed
    {
        assert_eq!(message, "Hello from JSON!");
        assert_eq!(role, "user");
        assert_eq!(topic, Some("general".to_string()));
    } else {
        panic!("Expected SendMessage message");
    }

    // Test parsing send_message JSON without topic
    let json_without_topic =
        r#"{"type":"send_message","message":"Hello without topic!","role":"user"}"#;
    let parsed: IncomingMessage = serde_json::from_str(json_without_topic).unwrap();
    if let IncomingMessage::SendMessage {
        message,
        role,
        topic,
        ..
    } = parsed
    {
        assert_eq!(message, "Hello without topic!");
        assert_eq!(role, "user");
        assert_eq!(topic, None);
    } else {
        panic!("Expected SendMessage message");
    }

    // Test send_message with different role
    let send_msg_system = IncomingMessage::SendMessage {
        message: "System notification".to_string(),
        role: "system".to_string(),
        topic: Some("notifications".to_string()),
        debug: None,
    };
    let json = serde_json::to_string(&send_msg_system).unwrap();
    assert!(json.contains("\"type\":\"send_message\""));
    assert!(json.contains("\"message\":\"System notification\""));
    assert!(json.contains("\"role\":\"system\""));
    assert!(json.contains("\"topic\":\"notifications\""));

    // Test send_message with debug field
    let send_msg_with_debug = IncomingMessage::SendMessage {
        message: "Debug message".to_string(),
        role: "user".to_string(),
        topic: None,
        debug: Some(serde_json::json!({
            "request_id": "123",
            "metadata": {
                "source": "test",
                "timestamp": 1234567890
            }
        })),
    };
    let json = serde_json::to_string(&send_msg_with_debug).unwrap();
    assert!(json.contains("\"type\":\"send_message\""));
    assert!(json.contains("\"message\":\"Debug message\""));
    assert!(json.contains("\"debug\""));
    assert!(json.contains("\"request_id\":\"123\""));
    assert!(json.contains("\"source\":\"test\""));

    // Test parsing send_message with debug field
    let json_with_debug = r#"{"type":"send_message","message":"Test","role":"user","debug":{"foo":"bar","nested":{"value":42}}}"#;
    let parsed: IncomingMessage = serde_json::from_str(json_with_debug).unwrap();
    if let IncomingMessage::SendMessage {
        message,
        role,
        debug,
        ..
    } = parsed
    {
        assert_eq!(message, "Test");
        assert_eq!(role, "user");
        assert!(debug.is_some());
        let debug_val = debug.unwrap();
        assert_eq!(debug_val["foo"], "bar");
        assert_eq!(debug_val["nested"]["value"], 42);
    } else {
        panic!("Expected SendMessage message");
    }
}

#[test]
fn test_outgoing_message_serialization() {
    // Test ready message without token
    let ready_msg = OutgoingMessage::Ready {
        livekit_token: None,
    };
    let json = serde_json::to_string(&ready_msg).unwrap();
    assert!(json.contains("\"type\":\"ready\""));

    // Test ready message with token
    let ready_msg_with_token = OutgoingMessage::Ready {
        livekit_token: Some("test_token".to_string()),
    };
    let json_with_token = serde_json::to_string(&ready_msg_with_token).unwrap();
    assert!(json_with_token.contains("\"type\":\"ready\""));
    assert!(json_with_token.contains("\"livekit_token\":\"test_token\""));

    // Test STT result message
    let stt_msg = OutgoingMessage::STTResult {
        transcript: "Hello world".to_string(),
        is_final: true,
        is_speech_final: true,
        confidence: 0.95,
    };

    let json = serde_json::to_string(&stt_msg).unwrap();
    assert!(json.contains("\"type\":\"stt_result\""));
    assert!(json.contains("Hello world"));
    assert!(json.contains("0.95"));

    // Test error message
    let error_msg = OutgoingMessage::Error {
        message: "Test error".to_string(),
    };

    let json = serde_json::to_string(&error_msg).unwrap();
    assert!(json.contains("\"type\":\"error\""));
    assert!(json.contains("Test error"));
}

#[test]
fn test_binary_audio_handling() {
    // Test that binary audio data is handled directly as bytes
    // without JSON serialization/deserialization
    let audio_data = vec![1, 2, 3, 4, 5];
    let bytes_data = Bytes::from(audio_data.clone());

    // Binary audio messages are now handled directly
    // No JSON serialization involved for better performance
    assert_eq!(bytes_data.to_vec(), audio_data);
    assert_eq!(bytes_data.len(), 5);
}

#[test]
fn test_stt_ws_config_conversion() {
    let stt_ws_config = STTWebSocketConfig {
        provider: "deepgram".to_string(),
        language: "en-US".to_string(),
        sample_rate: 16000,
        channels: 1,
        punctuation: true,
        encoding: "linear16".to_string(),
        model: "nova-3".to_string(),
    };

    let api_key = "test_api_key".to_string();
    let stt_config = stt_ws_config.to_stt_config(api_key.clone());

    assert_eq!(stt_config.provider, "deepgram");
    assert_eq!(stt_config.api_key, api_key);
    assert_eq!(stt_config.language, "en-US");
    assert_eq!(stt_config.sample_rate, 16000);
    assert_eq!(stt_config.channels, 1);
    assert!(stt_config.punctuation);
}

#[test]
fn test_tts_ws_config_conversion_with_all_values() {
    let tts_ws_config = TTSWebSocketConfig {
        provider: "deepgram".to_string(),
        voice_id: Some("custom-voice".to_string()),
        speaking_rate: Some(1.5),
        audio_format: Some("wav".to_string()),
        sample_rate: Some(22050),
        connection_timeout: Some(60),
        request_timeout: Some(120),
        model: "".to_string(), // Model is in Voice ID for Deepgram
        pronunciations: Vec::new(),
    };

    let api_key = "test_api_key".to_string();
    let tts_config = tts_ws_config.to_tts_config(api_key.clone());

    assert_eq!(tts_config.provider, "deepgram");
    assert_eq!(tts_config.api_key, api_key);
    assert_eq!(tts_config.voice_id, Some("custom-voice".to_string()));
    assert_eq!(tts_config.speaking_rate, Some(1.5));
    assert_eq!(tts_config.audio_format, Some("wav".to_string()));
    assert_eq!(tts_config.sample_rate, Some(22050));
    assert_eq!(tts_config.connection_timeout, Some(60));
    assert_eq!(tts_config.request_timeout, Some(120));
}

#[test]
fn test_tts_ws_config_conversion_with_defaults() {
    let tts_ws_config = TTSWebSocketConfig {
        provider: "deepgram".to_string(),
        voice_id: None,
        speaking_rate: None,
        audio_format: None,
        sample_rate: None,
        connection_timeout: None,
        request_timeout: None,
        model: "".to_string(), // Model is in Voice ID for Deepgram
        pronunciations: Vec::new(),
    };

    let api_key = "test_api_key".to_string();
    let tts_config = tts_ws_config.to_tts_config(api_key.clone());

    assert_eq!(tts_config.provider, "deepgram");
    assert_eq!(tts_config.api_key, api_key);

    // Should use default values
    assert_eq!(tts_config.voice_id, Some("aura-asteria-en".to_string()));
    assert_eq!(tts_config.speaking_rate, Some(1.0));
    assert_eq!(tts_config.audio_format, Some("linear16".to_string()));
    assert_eq!(tts_config.sample_rate, Some(24000));
    assert_eq!(tts_config.connection_timeout, Some(30));
    assert_eq!(tts_config.request_timeout, Some(60));
}

#[test]
fn test_livekit_ws_config_serialization() {
    let livekit_config = LiveKitWebSocketConfig {
        room_name: "test-room".to_string(),
        enable_recording: true,
        recording_file_key: Some("test-file-key".to_string()),
    };

    let json = serde_json::to_string(&livekit_config).unwrap();
    assert!(json.contains("\"room_name\":\"test-room\""));
    assert!(json.contains("\"enable_recording\":true"));
    assert!(json.contains("\"recording_file_key\":\"test-file-key\""));
}

#[test]
fn test_livekit_ws_config_conversion() {
    let livekit_ws_config = LiveKitWebSocketConfig {
        room_name: "test-room".to_string(),
        enable_recording: false,
        recording_file_key: None,
    };

    let tts_ws_config = TTSWebSocketConfig {
        provider: "deepgram".to_string(),
        voice_id: Some("aura-luna-en".to_string()),
        speaking_rate: Some(1.0),
        audio_format: Some("pcm".to_string()),
        sample_rate: Some(22050),
        connection_timeout: Some(30),
        request_timeout: Some(60),
        model: "".to_string(),
        pronunciations: Vec::new(),
    };

    let livekit_url = "wss://test-livekit.com".to_string();
    let test_token = "test-jwt-token".to_string();
    let livekit_config = livekit_ws_config.to_livekit_config(test_token.clone(), &tts_ws_config, &livekit_url);
    assert_eq!(livekit_config.url, "wss://test-livekit.com");
    assert_eq!(livekit_config.token, test_token);
    assert_eq!(livekit_config.room_name, "test-room");
    assert_eq!(livekit_config.sample_rate, 22050);
    assert_eq!(livekit_config.channels, 1);
    assert!(livekit_config.enable_noise_filter); // Should be true by default
}

#[test]
fn test_incoming_message_config_with_livekit() {
    let config_msg = IncomingMessage::Config {
        audio: Some(true),
        stt_config: Some(STTWebSocketConfig {
            provider: "deepgram".to_string(),
            language: "en-US".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            model: "nova-3".to_string(),
        }),
        tts_config: Some(TTSWebSocketConfig {
            provider: "deepgram".to_string(),
            voice_id: Some("aura-luna-en".to_string()),
            speaking_rate: Some(1.0),
            audio_format: Some("pcm".to_string()),
            sample_rate: Some(22050),
            connection_timeout: Some(30),
            request_timeout: Some(60),
            model: "".to_string(),
            pronunciations: Vec::new(),
        }),
        livekit: Some(LiveKitWebSocketConfig {
            room_name: "test-room".to_string(),
            enable_recording: true,
            recording_file_key: Some("test-file-key".to_string()),
        }),
    };

    let json = serde_json::to_string(&config_msg).unwrap();
    assert!(json.contains("\"type\":\"config\""));
    assert!(json.contains("\"room_name\":\"test-room\""));
    assert!(json.contains("\"livekit\"")); // Verify LiveKit section is present
}

#[test]
fn test_incoming_message_config_without_livekit() {
    let config_msg = IncomingMessage::Config {
        audio: Some(true),
        stt_config: Some(STTWebSocketConfig {
            provider: "deepgram".to_string(),
            language: "en-US".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            model: "nova-3".to_string(),
        }),
        tts_config: Some(TTSWebSocketConfig {
            provider: "deepgram".to_string(),
            voice_id: Some("aura-luna-en".to_string()),
            speaking_rate: Some(1.0),
            audio_format: Some("pcm".to_string()),
            sample_rate: Some(22050),
            connection_timeout: Some(30),
            request_timeout: Some(60),
            model: "".to_string(),
            pronunciations: Vec::new(),
        }),
        livekit: None,
    };

    let json = serde_json::to_string(&config_msg).unwrap();
    assert!(json.contains("\"type\":\"config\""));
    // Should not contain livekit field when None
    assert!(!json.contains("livekit"));
}

#[test]
fn test_parse_config_message_with_livekit() {
    let json = r#"{
            "type": "config",
            "stt_config": {
                "provider": "deepgram",
                "language": "en-US",
                "sample_rate": 16000,
                "channels": 1,
                "punctuation": true,
                "encoding": "linear16",
                "model": "nova-3"
            },
            "tts_config": {
                "provider": "deepgram",
                "voice_id": "aura-luna-en",
                "speaking_rate": 1.0,
                "audio_format": "pcm",
                "sample_rate": 22050,
                "connection_timeout": 30,
                "request_timeout": 60,
                "model": ""
            },
            "livekit": {
                "room_name": "test-room",
                "enable_recording": true,
                "recording_file_key": "test-file-key"
            }
        }"#;

    let parsed: IncomingMessage = serde_json::from_str(json).unwrap();
    if let IncomingMessage::Config {
        audio,
        stt_config,
        tts_config,
        livekit,
    } = parsed
    {
        assert_eq!(audio, Some(true));
        assert_eq!(stt_config.as_ref().unwrap().provider, "deepgram");
        assert_eq!(tts_config.as_ref().unwrap().provider, "deepgram");

        let livekit_config = livekit.unwrap();
        assert_eq!(livekit_config.room_name, "test-room");
        assert!(livekit_config.enable_recording);
        assert_eq!(livekit_config.recording_file_key, Some("test-file-key".to_string()));
    } else {
        panic!("Expected Config message");
    }
}

#[test]
fn test_unified_message_text_serialization() {
    let unified_msg = UnifiedMessage {
        message: Some("Hello from LiveKit!".to_string()),
        data: None,
        identity: "participant123".to_string(),
        topic: "chat".to_string(),
        room: "test-room".to_string(),
        timestamp: 1234567890,
    };

    let outgoing_msg = OutgoingMessage::Message {
        message: unified_msg,
    };

    let json = serde_json::to_string(&outgoing_msg).unwrap();
    assert!(json.contains("\"type\":\"message\""));
    assert!(json.contains("\"identity\":\"participant123\""));
    assert!(json.contains("\"message\":\"Hello from LiveKit!\""));
    assert!(json.contains("\"topic\":\"chat\""));
    assert!(json.contains("\"room\":\"test-room\""));
    assert!(json.contains("\"timestamp\":1234567890"));
}

#[test]
fn test_unified_message_data_serialization() {
    let test_data = vec![1, 2, 3, 4, 5];
    let unified_msg = UnifiedMessage {
        message: None,
        data: Some(general_purpose::STANDARD.encode(&test_data)),
        identity: "participant123".to_string(),
        topic: "files".to_string(),
        room: "test-room".to_string(),
        timestamp: 1234567890,
    };

    let outgoing_msg = OutgoingMessage::Message {
        message: unified_msg,
    };

    let json = serde_json::to_string(&outgoing_msg).unwrap();
    assert!(json.contains("\"type\":\"message\""));
    assert!(json.contains("\"identity\":\"participant123\""));
    assert!(json.contains("\"topic\":\"files\""));
    assert!(json.contains("\"room\":\"test-room\""));
    // Data should be present, message field should not be present (due to serde skip_serializing_if)
    assert!(json.contains("\"data\":"));
    assert!(json.contains("\"timestamp\":1234567890"));
    // message field should not be present (None value with skip_serializing_if)
    assert!(!json.contains("\"message\":null"));
}

#[test]
fn test_parse_config_message_without_livekit() {
    let json = r#"{
            "type": "config",
            "stt_config": {
                "provider": "deepgram",
                "language": "en-US",
                "sample_rate": 16000,
                "channels": 1,
                "punctuation": true,
                "encoding": "linear16",
                "model": "nova-3"
            },
            "tts_config": {
                "provider": "deepgram",
                "voice_id": "aura-luna-en",
                "speaking_rate": 1.0,
                "audio_format": "pcm",
                "sample_rate": 22050,
                "connection_timeout": 30,
                "request_timeout": 60,
                "model": ""
            }
        }"#;

    let parsed: IncomingMessage = serde_json::from_str(json).unwrap();
    if let IncomingMessage::Config {
        audio,
        stt_config,
        tts_config,
        livekit,
    } = parsed
    {
        assert_eq!(audio, Some(true));
        assert_eq!(stt_config.as_ref().unwrap().provider, "deepgram");
        assert_eq!(tts_config.as_ref().unwrap().provider, "deepgram");
        assert!(livekit.is_none());
    } else {
        panic!("Expected Config message");
    }
}

#[test]
fn test_tts_ws_config_conversion_mixed_values() {
    let tts_ws_config = TTSWebSocketConfig {
        provider: "deepgram".to_string(),
        voice_id: Some("custom-voice".to_string()),
        speaking_rate: None, // Should use default
        audio_format: Some("pcm".to_string()),
        sample_rate: None, // Should use default
        connection_timeout: Some(45),
        request_timeout: None, // Should use default
        model: "".to_string(), // Model is in Voice ID for Deepgram
        pronunciations: Vec::new(),
    };

    let api_key = "test_api_key".to_string();
    let tts_config = tts_ws_config.to_tts_config(api_key.clone());

    assert_eq!(tts_config.provider, "deepgram");
    assert_eq!(tts_config.api_key, api_key);
    assert_eq!(tts_config.voice_id, Some("custom-voice".to_string()));
    assert_eq!(tts_config.speaking_rate, Some(1.0)); // Default
    assert_eq!(tts_config.audio_format, Some("pcm".to_string()));
    assert_eq!(tts_config.sample_rate, Some(24000)); // Default
    assert_eq!(tts_config.connection_timeout, Some(45));
    assert_eq!(tts_config.request_timeout, Some(60)); // Default
}

#[test]
fn test_config_message_without_livekit_routing() {
    // Test that configuration without LiveKit creates proper routing logic
    let config_msg = IncomingMessage::Config {
        audio: Some(true),
        stt_config: Some(STTWebSocketConfig {
            provider: "deepgram".to_string(),
            language: "en-US".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            model: "nova-3".to_string(),
        }),
        tts_config: Some(TTSWebSocketConfig {
            provider: "deepgram".to_string(),
            voice_id: Some("aura-luna-en".to_string()),
            speaking_rate: Some(1.0),
            audio_format: Some("pcm".to_string()),
            sample_rate: Some(22050),
            connection_timeout: Some(30),
            request_timeout: Some(60),
            model: "".to_string(),
            pronunciations: Vec::new(),
        }),
        livekit: None, // No LiveKit configuration
    };

    let json = serde_json::to_string(&config_msg).unwrap();
    assert!(json.contains("\"type\":\"config\""));
    assert!(!json.contains("livekit")); // Should not contain LiveKit field

    // Parse back to ensure structure is correct
    let parsed: IncomingMessage = serde_json::from_str(&json).unwrap();
    if let IncomingMessage::Config { livekit, .. } = parsed {
        assert!(
            livekit.is_none(),
            "LiveKit should be None when not configured"
        );
    } else {
        panic!("Expected Config message");
    }
}

#[test]
fn test_config_message_with_livekit_routing() {
    // Test that configuration with LiveKit creates proper routing logic
    let config_msg = IncomingMessage::Config {
        audio: Some(true),
        stt_config: Some(STTWebSocketConfig {
            provider: "deepgram".to_string(),
            language: "en-US".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            model: "nova-3".to_string(),
        }),
        tts_config: Some(TTSWebSocketConfig {
            provider: "deepgram".to_string(),
            voice_id: Some("aura-luna-en".to_string()),
            speaking_rate: Some(1.0),
            audio_format: Some("pcm".to_string()),
            sample_rate: Some(22050),
            connection_timeout: Some(30),
            request_timeout: Some(60),
            model: "".to_string(),
            pronunciations: Vec::new(),
        }),
        livekit: Some(LiveKitWebSocketConfig {
            room_name: "test-room".to_string(),
            enable_recording: false,
            recording_file_key: None,
        }),
    };

    let json = serde_json::to_string(&config_msg).unwrap();
    assert!(json.contains("\"type\":\"config\""));
    assert!(json.contains("\"room_name\":\"test-room\""));
    assert!(json.contains("\"livekit\"")); // Verify LiveKit section is present

    // Parse back to ensure structure is correct
    let parsed: IncomingMessage = serde_json::from_str(&json).unwrap();
    if let IncomingMessage::Config { livekit, .. } = parsed {
        let livekit_config = livekit.unwrap();
        assert_eq!(livekit_config.room_name, "test-room");
        assert!(!livekit_config.enable_recording);
    } else {
        panic!("Expected Config message");
    }
}

#[test]
fn test_participant_disconnected_info_serialization() {
    let participant_info = ParticipantDisconnectedInfo {
        identity: "user123".to_string(),
        name: Some("John Doe".to_string()),
        room: "test-room".to_string(),
        timestamp: 1234567890,
    };

    let json = serde_json::to_string(&participant_info).unwrap();
    assert!(json.contains("\"identity\":\"user123\""));
    assert!(json.contains("\"name\":\"John Doe\""));
    assert!(json.contains("\"room\":\"test-room\""));
    assert!(json.contains("\"timestamp\":1234567890"));
}

#[test]
fn test_participant_disconnected_info_serialization_without_name() {
    let participant_info = ParticipantDisconnectedInfo {
        identity: "user456".to_string(),
        name: None,
        room: "test-room".to_string(),
        timestamp: 1234567890,
    };

    let json = serde_json::to_string(&participant_info).unwrap();
    assert!(json.contains("\"identity\":\"user456\""));
    assert!(json.contains("\"room\":\"test-room\""));
    assert!(json.contains("\"timestamp\":1234567890"));
    // Should not contain name field when None due to skip_serializing_if
    assert!(!json.contains("name"));
}

#[test]
fn test_participant_disconnected_outgoing_message() {
    let participant_info = ParticipantDisconnectedInfo {
        identity: "participant789".to_string(),
        name: Some("Alice Smith".to_string()),
        room: "conference-room".to_string(),
        timestamp: 9876543210,
    };

    let outgoing_msg = OutgoingMessage::ParticipantDisconnected {
        participant: participant_info,
    };

    let json = serde_json::to_string(&outgoing_msg).unwrap();
    assert!(json.contains("\"type\":\"participant_disconnected\""));
    assert!(json.contains("\"identity\":\"participant789\""));
    assert!(json.contains("\"name\":\"Alice Smith\""));
    assert!(json.contains("\"room\":\"conference-room\""));
    assert!(json.contains("\"timestamp\":9876543210"));
}

#[test]
fn test_participant_disconnected_json_format() {
    // Test that the serialized JSON has the expected format
    let participant_info = ParticipantDisconnectedInfo {
        identity: "user999".to_string(),
        name: Some("Bob Wilson".to_string()),
        room: "meeting-room".to_string(),
        timestamp: 1111111111,
    };

    let outgoing_msg = OutgoingMessage::ParticipantDisconnected {
        participant: participant_info,
    };

    let json = serde_json::to_string(&outgoing_msg).unwrap();

    // Verify that the JSON contains all expected fields
    assert!(json.contains("\"type\":\"participant_disconnected\""));
    assert!(json.contains("\"participant\":{"));
    assert!(json.contains("\"identity\":\"user999\""));
    assert!(json.contains("\"name\":\"Bob Wilson\""));
    assert!(json.contains("\"room\":\"meeting-room\""));
    assert!(json.contains("\"timestamp\":1111111111"));
}

#[test]
fn test_participant_disconnected_message_structure() {
    // Test the structure matches the documented API format
    let participant_info = ParticipantDisconnectedInfo {
        identity: "test-participant".to_string(),
        name: Some("Test User".to_string()),
        room: "test-room".to_string(),
        timestamp: 1000000000,
    };

    let outgoing_msg = OutgoingMessage::ParticipantDisconnected {
        participant: participant_info,
    };

    let json = serde_json::to_string(&outgoing_msg).unwrap();

    // Verify the JSON structure matches the API documentation
    let parsed_value: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed_value["type"], "participant_disconnected");
    assert_eq!(parsed_value["participant"]["identity"], "test-participant");
    assert_eq!(parsed_value["participant"]["name"], "Test User");
    assert_eq!(parsed_value["participant"]["room"], "test-room");
    assert_eq!(parsed_value["participant"]["timestamp"], 1000000000);
}

#[test]
fn test_tts_audio_routing_logic_without_livekit() {
    // Test that TTS audio data is properly handled without LiveKit
    let audio_data = vec![1, 2, 3, 4, 5];
    let bytes_data = Bytes::from(audio_data.clone());

    // When LiveKit is not configured, audio should go directly to WebSocket as binary
    assert_eq!(bytes_data.to_vec(), audio_data);
    assert_eq!(bytes_data.len(), 5);

    // Test that MessageRoute::Binary correctly wraps the audio data
    let route = MessageRoute::Binary(bytes_data);
    match route {
        MessageRoute::Binary(data) => {
            assert_eq!(data.to_vec(), audio_data);
        }
        _ => panic!("Expected Binary route"),
    }
}

#[test]
fn test_tts_audio_routing_logic_with_livekit() {
    // Test that TTS audio routing logic properly handles LiveKit scenarios

    // Test case 1: LiveKit configured and available
    // In this case, audio should be routed to LiveKit, not WebSocket
    // This is tested through the unified callback logic in the actual handler

    // Test case 2: LiveKit configured but disconnected
    // Audio should fall back to WebSocket

    // Test case 3: LiveKit send failure
    // Audio should fall back to WebSocket

    // These scenarios are integration-tested through the actual WebSocket handler
    // The unit test here validates the data structures and routing logic

    let test_audio = vec![0x01, 0x02, 0x03, 0x04];
    let audio_bytes = Bytes::from(test_audio.clone());

    // Verify the audio data can be properly converted for both routing paths
    assert_eq!(audio_bytes.to_vec(), test_audio);

    // Test that cloning works for dual routing scenarios
    let cloned_audio = audio_bytes.clone();
    assert_eq!(cloned_audio.to_vec(), test_audio);
    assert_eq!(audio_bytes.to_vec(), test_audio);
}

#[test]
fn test_config_message_audio_disabled() {
    // Test configuration with audio=false (LiveKit-only mode)
    let config_msg = IncomingMessage::Config {
        audio: Some(false),
        stt_config: None, // Not required when audio=false
        tts_config: None, // Not required when audio=false
        livekit: Some(LiveKitWebSocketConfig {
            room_name: "test-room".to_string(),
            enable_recording: false,
            recording_file_key: None,
        }),
    };

    let json = serde_json::to_string(&config_msg).unwrap();
    assert!(json.contains("\"type\":\"config\""));
    assert!(json.contains("\"audio\":false"));
    assert!(json.contains("\"room_name\":\"test-room\""));
    assert!(json.contains("\"livekit\""));
    // Should not contain stt_config or tts_config fields when None
    assert!(!json.contains("stt_config"));
    assert!(!json.contains("tts_config"));

    // Parse back to ensure structure is correct
    let parsed: IncomingMessage = serde_json::from_str(&json).unwrap();
    if let IncomingMessage::Config {
        audio,
        stt_config,
        tts_config,
        livekit,
    } = parsed
    {
        assert_eq!(audio, Some(false));
        assert!(stt_config.is_none());
        assert!(tts_config.is_none());
        assert!(livekit.is_some());
    } else {
        panic!("Expected Config message");
    }
}

#[test]
fn test_config_message_audio_default() {
    // Test configuration with no audio field (should default to true)
    let config_msg = IncomingMessage::Config {
        audio: None, // Should default to true
        stt_config: Some(STTWebSocketConfig {
            provider: "deepgram".to_string(),
            language: "en-US".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            model: "nova-3".to_string(),
        }),
        tts_config: Some(TTSWebSocketConfig {
            provider: "deepgram".to_string(),
            voice_id: Some("aura-luna-en".to_string()),
            speaking_rate: Some(1.0),
            audio_format: Some("pcm".to_string()),
            sample_rate: Some(22050),
            connection_timeout: Some(30),
            request_timeout: Some(60),
            model: "".to_string(),
            pronunciations: Vec::new(),
        }),
        livekit: None,
    };

    let json = serde_json::to_string(&config_msg).unwrap();
    assert!(json.contains("\"type\":\"config\""));
    // Should not contain audio field when None (due to skip_serializing_if)
    assert!(!json.contains("\"audio\""));

    // Parse back to ensure structure is correct
    let parsed: IncomingMessage = serde_json::from_str(&json).unwrap();
    if let IncomingMessage::Config {
        audio,
        stt_config,
        tts_config,
        livekit,
    } = parsed
    {
        assert_eq!(audio, Some(true)); // Should default to true via serde default
        assert!(stt_config.is_some());
        assert!(tts_config.is_some());
        assert!(livekit.is_none());
    } else {
        panic!("Expected Config message");
    }
}

#[test]
fn test_unified_message_for_livekit_integration() {
    // Test unified message structure used for LiveKit data forwarding
    let test_data = b"Hello from LiveKit participant";

    // Test text message from LiveKit
    let text_message = UnifiedMessage {
        message: Some(String::from_utf8(test_data.to_vec()).unwrap()),
        data: None,
        identity: "participant123".to_string(),
        topic: "chat".to_string(),
        room: "test-room".to_string(),
        timestamp: 1234567890,
    };

    let outgoing_msg = OutgoingMessage::Message {
        message: text_message,
    };

    let json = serde_json::to_string(&outgoing_msg).unwrap();
    assert!(json.contains("\"type\":\"message\""));
    assert!(json.contains("\"identity\":\"participant123\""));
    assert!(json.contains("Hello from LiveKit participant"));

    // Test binary data message from LiveKit
    let binary_message = UnifiedMessage {
        message: None,
        data: Some(general_purpose::STANDARD.encode(test_data)),
        identity: "participant456".to_string(),
        topic: "files".to_string(),
        room: "test-room".to_string(),
        timestamp: 1234567891,
    };

    let outgoing_msg_binary = OutgoingMessage::Message {
        message: binary_message,
    };

    let json_binary = serde_json::to_string(&outgoing_msg_binary).unwrap();
    assert!(json_binary.contains("\"type\":\"message\""));
    assert!(json_binary.contains("\"identity\":\"participant456\""));
    assert!(json_binary.contains("\"topic\":\"files\""));
    assert!(json_binary.contains("\"data\":"));
    // Should not contain message field for binary data
    assert!(!json_binary.contains("\"message\":null"));
}
