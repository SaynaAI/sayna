//! Timing control tests for speech_final handling.
//!
//! Tests verify is_final results set the waiting flag and real speech_final
//! events properly reset state.

use crate::core::stt::STTResult;
use crate::core::voice_manager::stt_result::STTResultProcessor;
use crate::core::voice_manager::tests::helpers::{
    create_voice_manager, default_voice_manager_config,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::mpsc;

use super::helpers::{IS_FINAL_TEST_CASES, new_speech_final_state, reset_speech_final_state};

#[tokio::test]
async fn test_is_final_sets_waiting_flag() {
    for (i, test_case) in IS_FINAL_TEST_CASES.iter().enumerate() {
        let speech_final_state = new_speech_final_state();
        reset_speech_final_state(&speech_final_state);

        let result = STTResult::new(
            test_case.transcript.to_string(),
            true,
            false,
            test_case.confidence,
        );
        let processor = STTResultProcessor::default();
        let processed = processor
            .process_result(result, speech_final_state.clone())
            .await;

        assert!(
            processed.is_some(),
            "Test case {}: should return result",
            i + 1
        );
        let processed_result = processed.unwrap();
        assert_eq!(
            processed_result.transcript,
            test_case.transcript,
            "Test case {}: transcript mismatch",
            i + 1
        );
        assert!(
            processed_result.is_final,
            "Test case {}: should be final",
            i + 1
        );
        assert!(
            !processed_result.is_speech_final,
            "Test case {}: should not be speech_final",
            i + 1
        );

        let state = speech_final_state.read();
        assert!(
            state.waiting_for_speech_final.load(Ordering::Acquire),
            "Test case {}: should be waiting for speech_final",
            i + 1
        );
        assert_eq!(
            state.text_buffer,
            test_case.transcript,
            "Test case {}: text_buffer mismatch",
            i + 1
        );
    }
}

#[tokio::test]
async fn test_real_speech_final_resets_state() {
    let speech_final_state = new_speech_final_state();
    reset_speech_final_state(&speech_final_state);

    let result1 = STTResult::new("Hello world".to_string(), true, false, 0.9);
    let processor = STTResultProcessor::default();
    let _processed1 = processor
        .process_result(result1, speech_final_state.clone())
        .await;

    {
        let state = speech_final_state.read();
        assert!(state.waiting_for_speech_final.load(Ordering::Acquire));
        assert_eq!(state.text_buffer, "Hello world");
    }

    let result2 = STTResult::new("final result".to_string(), true, true, 0.95);
    let processor2 = STTResultProcessor::default();
    let processed2 = processor2
        .process_result(result2, speech_final_state.clone())
        .await;

    assert!(processed2.is_some());
    let final_result = processed2.unwrap();
    assert!(final_result.is_speech_final);
    assert!(final_result.is_final);
    assert_eq!(final_result.transcript, "final result");
    assert_eq!(final_result.confidence, 0.95);

    let state = speech_final_state.read();
    assert!(!state.waiting_for_speech_final.load(Ordering::Acquire));
    assert!(state.text_buffer.is_empty());
}

#[tokio::test]
async fn test_direct_speech_final_returns_original() {
    let speech_final_state = new_speech_final_state();
    reset_speech_final_state(&speech_final_state);

    let result = STTResult::new("Direct speech final".to_string(), true, true, 0.85);
    let processor = STTResultProcessor::default();
    let processed = processor
        .process_result(result, speech_final_state.clone())
        .await;

    assert!(processed.is_some());
    let final_result = processed.unwrap();
    assert!(final_result.is_speech_final);
    assert!(final_result.is_final);
    assert_eq!(final_result.transcript, "Direct speech final");
    assert_eq!(final_result.confidence, 0.85);

    let state = speech_final_state.read();
    assert!(!state.waiting_for_speech_final.load(Ordering::Acquire));
    assert!(state.text_buffer.is_empty());
}

#[tokio::test]
async fn test_voice_manager_callback_registration() {
    let config = default_voice_manager_config();
    let voice_manager = create_voice_manager!(config, None).unwrap();

    let (tx, _rx) = mpsc::unbounded_channel();
    let result_counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = result_counter.clone();

    voice_manager
        .on_stt_result(move |result| {
            let tx = tx.clone();
            let counter = counter_clone.clone();
            Box::pin(async move {
                counter.fetch_add(1, Ordering::Relaxed);
                let _ = tx.send(result);
            })
        })
        .await
        .unwrap();
}
