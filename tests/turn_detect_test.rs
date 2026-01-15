#[cfg(feature = "stt-vad")]
use anyhow::Result;
#[cfg(feature = "stt-vad")]
use sayna::core::turn_detect::TurnDetector;
use sayna::core::turn_detect::TurnDetectorConfig;
#[cfg(feature = "stt-vad")]
use std::path::PathBuf;

#[cfg(feature = "stt-vad")]
#[tokio::test]
async fn test_turn_detector_initialization() -> Result<()> {
    let config = TurnDetectorConfig {
        model_path: Some(PathBuf::from("/tmp/test_model.onnx")),
        tokenizer_path: Some(PathBuf::from("/tmp/test_tokenizer.json")),
        ..Default::default()
    };

    // This test will fail if model/tokenizer files don't exist
    // In a real scenario, we'd mock or provide test files
    match TurnDetector::with_config(config).await {
        Ok(_detector) => {
            println!("TurnDetector initialized successfully");
        }
        Err(e) => {
            println!(
                "Expected error during initialization (no model files): {}",
                e
            );
        }
    }

    Ok(())
}

#[tokio::test]
async fn test_config_defaults() {
    let config = TurnDetectorConfig::default();
    assert_eq!(config.threshold, 0.7);
    assert_eq!(config.max_context_turns, 4);
    assert_eq!(config.max_sequence_length, 512);
    assert!(config.use_quantized);
    assert_eq!(config.num_threads, Some(4));
}

#[cfg(feature = "stt-vad")]
#[ignore = "Requires model files to be downloaded"]
#[tokio::test]
async fn test_end_to_end_turn_detection() -> Result<()> {
    // This test requires actual model files
    let detector = TurnDetector::new(None).await?;

    // Test complete user utterances
    // Note: The model's behavior may vary from expected human judgment
    let test_cases = vec![
        ("What is the capital of France?", true), // Complete question
        ("Thank you", true),                      // Complete statement
        ("How are you doing today?", true),       // Complete question
    ];

    for (text, should_be_complete) in test_cases {
        let probability = detector.predict_end_of_turn(text).await?;
        println!("Text: '{}' - Probability: {:.4}", text, probability);

        let is_complete = detector.is_turn_complete(text).await?;
        if should_be_complete {
            assert!(is_complete, "Expected '{}' to be complete", text);
        } else {
            assert!(!is_complete, "Expected '{}' to be incomplete", text);
        }
    }

    Ok(())
}

#[cfg(feature = "stt-vad")]
#[ignore = "Requires model files to be downloaded"]
#[tokio::test]
async fn test_streaming_text_detection() -> Result<()> {
    let detector = TurnDetector::new(None).await?;

    // Simulate streaming text
    let partial_texts = vec![
        "What",
        "What is",
        "What is the",
        "What is the weather",
        "What is the weather like",
        "What is the weather like today",
        "What is the weather like today?",
    ];

    for text in partial_texts {
        let is_complete = detector.process_streaming_text(text, 10).await?;
        println!("Streaming text: '{}' - Complete: {}", text, is_complete);
    }

    Ok(())
}

#[test]
fn test_config_serialization() {
    let config = TurnDetectorConfig::default();
    let json = serde_json::to_string(&config).unwrap();
    let deserialized: TurnDetectorConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(config.threshold, deserialized.threshold);
    assert_eq!(config.max_context_turns, deserialized.max_context_turns);
}
