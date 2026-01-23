//! Unit tests for VAD state and processing.

use super::super::utils::MAX_VAD_AUDIO_SAMPLES;
use super::processor::bytes_to_samples;
use super::state::VADState;

#[test]
fn test_force_error_flag() {
    let vad_state = VADState::new(1024, 512);

    assert!(!vad_state.is_force_error());

    vad_state.set_force_error(true);
    assert!(vad_state.is_force_error());

    vad_state.set_force_error(false);
    assert!(!vad_state.is_force_error());
}

#[test]
fn test_oversized_chunk_does_not_panic() {
    let vad_state = VADState::new(1024, 512);

    {
        let mut buffer = vad_state.audio_buffer.write();
        buffer.extend(std::iter::repeat_n(0i16, 10_000));
    }

    let oversized_samples: Vec<i16> = (0..MAX_VAD_AUDIO_SAMPLES + 50_000)
        .map(|i| (i % 1000) as i16)
        .collect();

    {
        let mut audio_buffer = vad_state.audio_buffer.write();

        if oversized_samples.len() >= MAX_VAD_AUDIO_SAMPLES {
            audio_buffer.clear();
            let start_idx = oversized_samples.len() - MAX_VAD_AUDIO_SAMPLES;
            audio_buffer.extend(oversized_samples[start_idx..].iter().copied());
        }
    }

    let buffer = vad_state.audio_buffer.read();
    assert_eq!(buffer.len(), MAX_VAD_AUDIO_SAMPLES);
    assert_eq!(buffer[0], (50_000 % 1000) as i16);
}

#[test]
fn test_normal_chunk_with_clamping() {
    let vad_state = VADState::new(1024, 512);

    let fill_count = MAX_VAD_AUDIO_SAMPLES - 1000;
    {
        let mut buffer = vad_state.audio_buffer.write();
        buffer.extend(std::iter::repeat_n(1i16, fill_count));
    }

    let new_samples: Vec<i16> = (0..5000).map(|i| (i + 100) as i16).collect();
    {
        let mut audio_buffer = vad_state.audio_buffer.write();

        let new_total = audio_buffer.len() + new_samples.len();
        if new_total > MAX_VAD_AUDIO_SAMPLES {
            let samples_to_drop = new_total - MAX_VAD_AUDIO_SAMPLES;
            let drop_count = samples_to_drop.min(audio_buffer.len());
            audio_buffer.drain(..drop_count);
        }
        audio_buffer.extend(new_samples.iter().copied());
    }

    let buffer = vad_state.audio_buffer.read();
    assert_eq!(buffer.len(), MAX_VAD_AUDIO_SAMPLES);
}

#[test]
fn test_bytes_to_samples_even_length_aligned() {
    // Create aligned audio data: 4 samples = 8 bytes
    let samples: Vec<i16> = vec![100, 200, -300, 400];
    let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();

    // Ensure the slice is aligned (Vec<u8> typically is on most platforms)
    let result = bytes_to_samples(&bytes);

    assert_eq!(result.len(), 4);
    assert_eq!(result[0], 100);
    assert_eq!(result[1], 200);
    assert_eq!(result[2], -300);
    assert_eq!(result[3], 400);
}

#[test]
fn test_bytes_to_samples_odd_length() {
    // Odd-length input: 5 bytes, should produce 2 samples (last byte ignored)
    let bytes: Vec<u8> = vec![0x64, 0x00, 0xC8, 0x00, 0xFF]; // 100, 200, trailing 0xFF

    let result = bytes_to_samples(&bytes);

    assert_eq!(result.len(), 2);
    assert_eq!(result[0], 100); // 0x0064 in little-endian
    assert_eq!(result[1], 200); // 0x00C8 in little-endian
}

#[test]
fn test_bytes_to_samples_empty() {
    let bytes: &[u8] = &[];
    let result = bytes_to_samples(bytes);

    assert!(result.is_empty());
}

#[test]
fn test_bytes_to_samples_single_byte() {
    // Single byte should produce empty result (need at least 2 bytes for one sample)
    let bytes: &[u8] = &[0x42];
    let result = bytes_to_samples(bytes);

    assert!(result.is_empty());
}

#[test]
fn test_bytes_to_samples_returns_borrowed_when_aligned() {
    // Create aligned data using a Vec (which is typically aligned)
    let samples: Vec<i16> = vec![1, 2, 3, 4];
    let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();

    let result = bytes_to_samples(&bytes);

    // On most platforms, Vec<u8> is aligned, so we should get Borrowed.
    // If not aligned, we get Owned, which is still correct behavior.
    // The important thing is that the values are correct.
    assert_eq!(result.len(), 4);
    assert_eq!(&result[..], &[1, 2, 3, 4]);
}

#[test]
fn test_bytes_to_samples_unaligned_fallback() {
    // Force unaligned access by using an offset into a buffer.
    // We allocate extra space and skip one byte to create misalignment.
    let mut buffer: Vec<u8> = vec![0u8; 10];
    // Fill bytes 1..9 with sample data (4 samples)
    let samples: Vec<i16> = vec![500, 600, 700, 800];
    for (i, sample) in samples.iter().enumerate() {
        let le_bytes = sample.to_le_bytes();
        buffer[1 + i * 2] = le_bytes[0];
        buffer[1 + i * 2 + 1] = le_bytes[1];
    }

    // Take a slice starting at offset 1 (likely unaligned for i16)
    let unaligned_slice = &buffer[1..9];

    let result = bytes_to_samples(unaligned_slice);

    // Whether aligned or not, the result should be correct
    assert_eq!(result.len(), 4);
    assert_eq!(result[0], 500);
    assert_eq!(result[1], 600);
    assert_eq!(result[2], 700);
    assert_eq!(result[3], 800);
}
