//! Duplicate speech_final prevention tests.
//!
//! Tests verify that duplicate speech_final events are properly suppressed
//! when VAD turn detection has already fired.

use crate::core::stt::STTResult;
use crate::core::voice_manager::stt_result::STTResultProcessor;
use std::sync::atomic::Ordering;

use super::helpers::{new_speech_final_state, simulate_vad_turn_detection};

#[tokio::test]
async fn test_forced_speech_final_then_real_prevents_duplicate() {
    let speech_final_state = new_speech_final_state();

    let result1 = STTResult::new("Hello world".to_string(), true, false, 0.9);
    let processor1 = STTResultProcessor::default();
    let processed1 = processor1
        .process_result(result1.clone(), speech_final_state.clone())
        .await;

    assert!(processed1.is_some());
    assert_eq!(processed1.unwrap().transcript, "Hello world");
    assert!(
        speech_final_state
            .read()
            .waiting_for_speech_final
            .load(Ordering::Acquire)
    );

    simulate_vad_turn_detection(&speech_final_state, "Hello world", 0);

    let result2 = STTResult::new("Hello world".to_string(), true, true, 0.95);
    let processor2 = STTResultProcessor::default();
    let processed2 = processor2
        .process_result(result2, speech_final_state.clone())
        .await;

    assert!(processed2.is_none());
}

#[tokio::test]
async fn test_multiple_is_final_continues_accumulating() {
    let speech_final_state = new_speech_final_state();

    let result1 = STTResult::new("First".to_string(), true, false, 0.9);
    let processor1 = STTResultProcessor::default();
    let processed1 = processor1
        .process_result(result1, speech_final_state.clone())
        .await;

    assert!(processed1.is_some());

    simulate_vad_turn_detection(&speech_final_state, "First", -1000);

    let result2 = STTResult::new("Second".to_string(), true, false, 0.9);
    let processor2 = STTResultProcessor::default();
    let processed2 = processor2
        .process_result(result2, speech_final_state.clone())
        .await;

    assert!(processed2.is_some());
    assert_eq!(processed2.unwrap().transcript, "Second");
    assert!(
        speech_final_state
            .read()
            .waiting_for_speech_final
            .load(Ordering::Acquire)
    );
}

#[tokio::test]
async fn test_new_segment_after_reset_works_normally() {
    let speech_final_state = new_speech_final_state();

    let result1 = STTResult::new("First segment".to_string(), true, false, 0.9);
    let processor1 = STTResultProcessor::default();
    let processed1 = processor1
        .process_result(result1, speech_final_state.clone())
        .await;
    assert!(processed1.is_some());

    simulate_vad_turn_detection(&speech_final_state, "First segment", 0);

    let result2 = STTResult::new("First segment".to_string(), true, true, 0.9);
    let processor2 = STTResultProcessor::default();
    let processed2 = processor2
        .process_result(result2, speech_final_state.clone())
        .await;
    assert!(processed2.is_none());

    {
        let mut state = speech_final_state.write();
        state.text_buffer.clear();
        state
            .waiting_for_speech_final
            .store(false, Ordering::Release);
    }

    let new_result = STTResult::new("New segment".to_string(), true, false, 0.9);
    let processor_new = STTResultProcessor::default();
    let processed_new = processor_new
        .process_result(new_result, speech_final_state.clone())
        .await;

    assert!(processed_new.is_some());
    assert_eq!(processed_new.unwrap().transcript, "New segment");
    assert!(
        speech_final_state
            .read()
            .waiting_for_speech_final
            .load(Ordering::Acquire)
    );
}
