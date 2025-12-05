//! Voice Activity Detection (VAD) for Whisper real-time streaming
//!
//! This module provides simple energy-based voice activity detection to enable
//! real-time streaming transcription with Whisper. It detects speech boundaries
//! to determine when to:
//! - Start buffering audio for transcription
//! - Emit interim results during speech
//! - Mark `is_speech_final` when the user stops speaking
//!
//! ## Algorithm
//!
//! Uses RMS (Root Mean Square) energy detection:
//! 1. Compute RMS energy of audio frame
//! 2. Compare against adaptive threshold
//! 3. Track consecutive silent/speech frames
//! 4. Emit state transitions based on hysteresis
//!
//! ## Reference
//!
//! Based on whisper.cpp streaming VAD approach:
//! https://github.com/ggml-org/whisper.cpp/blob/master/examples/stream/README.md

#![cfg(feature = "whisper-stt")]

use tracing::debug;

use super::mel::SAMPLE_RATE;

/// VAD configuration parameters
#[derive(Debug, Clone)]
pub struct VADConfig {
    /// Energy threshold for speech detection (0.0 to 1.0)
    /// Higher values require louder speech to trigger
    /// Default: 0.01 (suitable for typical speech)
    pub energy_threshold: f32,

    /// Duration of silence required to mark speech as final (ms)
    /// Default: 500ms - good balance between responsiveness and false triggers
    pub silence_duration_ms: u32,

    /// Minimum speech duration before considering it valid (ms)
    /// Filters out short noise bursts
    /// Default: 250ms
    pub min_speech_duration_ms: u32,

    /// Frame size for energy calculation (samples)
    /// Default: 1600 (100ms at 16kHz)
    pub frame_size: usize,
}

impl Default for VADConfig {
    fn default() -> Self {
        Self {
            energy_threshold: 0.01,
            silence_duration_ms: 500,
            min_speech_duration_ms: 250,
            frame_size: 1600, // 100ms at 16kHz
        }
    }
}

impl VADConfig {
    /// Configuration optimized for quick response (may have more false positives)
    pub fn responsive() -> Self {
        Self {
            energy_threshold: 0.008,
            silence_duration_ms: 300,
            min_speech_duration_ms: 150,
            frame_size: 800, // 50ms
        }
    }

    /// Configuration optimized for accuracy (may be slower to respond)
    pub fn accurate() -> Self {
        Self {
            energy_threshold: 0.015,
            silence_duration_ms: 700,
            min_speech_duration_ms: 400,
            frame_size: 1600, // 100ms
        }
    }

    /// Convert duration in ms to sample count
    fn duration_to_samples(&self, duration_ms: u32) -> usize {
        ((duration_ms as u64 * SAMPLE_RATE as u64) / 1000) as usize
    }
}

/// Voice Activity Detection state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VADState {
    /// No speech detected, waiting for speech to start
    Silence,
    /// Speech is active
    Speaking,
    /// Speech just ended (silence detected after speech)
    SpeechEnded,
}

/// Voice Activity Detector
///
/// Tracks audio energy levels to detect speech boundaries.
/// Used by WhisperSTT to implement real-time streaming.
#[derive(Debug)]
pub struct VoiceActivityDetector {
    /// Configuration
    config: VADConfig,
    /// Current VAD state
    state: VADState,
    /// Count of consecutive silent frames
    silent_frames: usize,
    /// Count of consecutive speech frames
    speech_frames: usize,
    /// Total samples seen in current speech segment
    speech_samples: usize,
    /// Running average energy for adaptive thresholding
    avg_energy: f32,
    /// Number of frames used for averaging
    energy_frame_count: usize,
}

impl VoiceActivityDetector {
    /// Create a new VAD with default configuration
    pub fn new() -> Self {
        Self::with_config(VADConfig::default())
    }

    /// Create a new VAD with custom configuration
    pub fn with_config(config: VADConfig) -> Self {
        Self {
            config,
            state: VADState::Silence,
            silent_frames: 0,
            speech_frames: 0,
            speech_samples: 0,
            avg_energy: 0.0,
            energy_frame_count: 0,
        }
    }

    /// Process an audio frame and return the current VAD state
    ///
    /// # Arguments
    /// * `audio` - Audio samples (f32, normalized to [-1.0, 1.0])
    ///
    /// # Returns
    /// Current VAD state after processing
    pub fn process(&mut self, audio: &[f32]) -> VADState {
        if audio.is_empty() {
            return self.state;
        }

        // Process audio in frames
        for frame in audio.chunks(self.config.frame_size) {
            self.process_frame(frame);
        }

        self.state
    }

    /// Process a single audio frame
    fn process_frame(&mut self, frame: &[f32]) {
        let energy = Self::compute_rms_energy(frame);

        // Update running average energy (for potential adaptive thresholding)
        self.update_avg_energy(energy);

        let is_speech = energy > self.config.energy_threshold;

        match self.state {
            VADState::Silence => {
                if is_speech {
                    self.speech_frames += 1;
                    self.speech_samples += frame.len();

                    // Require minimum consecutive speech frames to start
                    let min_frames = 2; // At least 2 frames of speech
                    if self.speech_frames >= min_frames {
                        debug!(
                            energy = energy,
                            threshold = self.config.energy_threshold,
                            "VAD: Speech started"
                        );
                        self.state = VADState::Speaking;
                    }
                } else {
                    self.speech_frames = 0;
                    self.speech_samples = 0;
                }
            }
            VADState::Speaking => {
                self.speech_samples += frame.len();

                if is_speech {
                    self.silent_frames = 0;
                } else {
                    self.silent_frames += 1;

                    // Calculate how many silent frames needed for speech end
                    let silence_samples = self
                        .config
                        .duration_to_samples(self.config.silence_duration_ms);
                    let silence_frames_needed = silence_samples / self.config.frame_size;

                    if self.silent_frames >= silence_frames_needed {
                        // Check if speech was long enough to be valid
                        let min_samples = self
                            .config
                            .duration_to_samples(self.config.min_speech_duration_ms);

                        if self.speech_samples >= min_samples {
                            debug!(
                                silent_frames = self.silent_frames,
                                speech_samples = self.speech_samples,
                                "VAD: Speech ended"
                            );
                            self.state = VADState::SpeechEnded;
                        } else {
                            // Speech was too short, treat as noise
                            debug!(
                                speech_samples = self.speech_samples,
                                min_samples = min_samples,
                                "VAD: Speech too short, ignoring"
                            );
                            self.reset();
                        }
                    }
                }
            }
            VADState::SpeechEnded => {
                // SpeechEnded is a transient state - move to Silence after acknowledgment
                if is_speech {
                    // New speech started immediately
                    self.state = VADState::Speaking;
                    self.silent_frames = 0;
                    self.speech_frames = 1;
                    self.speech_samples = frame.len();
                } else {
                    // Stay in ended state until reset is called
                    // This allows the client to detect the transition
                }
            }
        }
    }

    /// Compute RMS (Root Mean Square) energy of audio frame
    fn compute_rms_energy(audio: &[f32]) -> f32 {
        if audio.is_empty() {
            return 0.0;
        }

        let sum_squares: f32 = audio.iter().map(|s| s * s).sum();
        (sum_squares / audio.len() as f32).sqrt()
    }

    /// Update running average energy
    fn update_avg_energy(&mut self, energy: f32) {
        // Exponential moving average with decay factor
        const DECAY: f32 = 0.95;

        if self.energy_frame_count == 0 {
            self.avg_energy = energy;
        } else {
            self.avg_energy = DECAY * self.avg_energy + (1.0 - DECAY) * energy;
        }
        self.energy_frame_count += 1;
    }

    /// Reset VAD state (call after processing speech end)
    pub fn reset(&mut self) {
        self.state = VADState::Silence;
        self.silent_frames = 0;
        self.speech_frames = 0;
        self.speech_samples = 0;
    }

    /// Get current VAD state
    pub fn state(&self) -> VADState {
        self.state
    }

    /// Check if currently speaking
    pub fn is_speaking(&self) -> bool {
        self.state == VADState::Speaking
    }

    /// Check if speech just ended
    pub fn is_speech_ended(&self) -> bool {
        self.state == VADState::SpeechEnded
    }

    /// Get the duration of current speech segment in milliseconds
    pub fn speech_duration_ms(&self) -> u64 {
        ((self.speech_samples as u64) * 1000) / (SAMPLE_RATE as u64)
    }

    /// Get the current energy threshold
    pub fn energy_threshold(&self) -> f32 {
        self.config.energy_threshold
    }

    /// Get the average energy level seen
    pub fn avg_energy(&self) -> f32 {
        self.avg_energy
    }

    /// Get the configuration
    pub fn config(&self) -> &VADConfig {
        &self.config
    }
}

impl Default for VoiceActivityDetector {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper function to check if an audio buffer contains speech
///
/// This is a stateless convenience function for simple use cases.
pub fn is_speech(audio: &[f32], threshold: f32) -> bool {
    let energy = VoiceActivityDetector::compute_rms_energy(audio);
    energy > threshold
}

/// Helper function to compute the RMS energy of an audio buffer
pub fn compute_energy(audio: &[f32]) -> f32 {
    VoiceActivityDetector::compute_rms_energy(audio)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vad_config_default() {
        let config = VADConfig::default();
        assert_eq!(config.energy_threshold, 0.01);
        assert_eq!(config.silence_duration_ms, 500);
        assert_eq!(config.min_speech_duration_ms, 250);
        assert_eq!(config.frame_size, 1600);
    }

    #[test]
    fn test_vad_config_responsive() {
        let config = VADConfig::responsive();
        assert!(config.silence_duration_ms < VADConfig::default().silence_duration_ms);
        assert!(config.min_speech_duration_ms < VADConfig::default().min_speech_duration_ms);
    }

    #[test]
    fn test_vad_config_accurate() {
        let config = VADConfig::accurate();
        assert!(config.silence_duration_ms > VADConfig::default().silence_duration_ms);
        assert!(config.min_speech_duration_ms > VADConfig::default().min_speech_duration_ms);
    }

    #[test]
    fn test_vad_new() {
        let vad = VoiceActivityDetector::new();
        assert_eq!(vad.state(), VADState::Silence);
        assert!(!vad.is_speaking());
        assert!(!vad.is_speech_ended());
    }

    #[test]
    fn test_vad_silence_detection() {
        let mut vad = VoiceActivityDetector::new();

        // Process silent audio (near zero)
        let silent_audio = vec![0.001f32; 1600];
        let state = vad.process(&silent_audio);

        assert_eq!(state, VADState::Silence);
        assert!(!vad.is_speaking());
    }

    #[test]
    fn test_vad_speech_detection() {
        let mut vad = VoiceActivityDetector::new();

        // Process loud audio (above threshold)
        let loud_audio = vec![0.1f32; 3200]; // 2 frames worth

        let state = vad.process(&loud_audio);

        assert_eq!(state, VADState::Speaking);
        assert!(vad.is_speaking());
    }

    #[test]
    fn test_vad_speech_end_detection() {
        let config = VADConfig {
            energy_threshold: 0.01,
            silence_duration_ms: 100,   // Short for testing
            min_speech_duration_ms: 50, // Short for testing
            frame_size: 160,            // Small frame for testing
        };
        let mut vad = VoiceActivityDetector::with_config(config);

        // Start with speech
        let loud_audio = vec![0.1f32; 320]; // 2 frames
        vad.process(&loud_audio);
        assert!(vad.is_speaking());

        // Follow with silence (enough for speech end)
        let silent_audio = vec![0.001f32; 3200]; // Many silent frames
        let state = vad.process(&silent_audio);

        assert_eq!(state, VADState::SpeechEnded);
        assert!(vad.is_speech_ended());
    }

    #[test]
    fn test_vad_reset() {
        let mut vad = VoiceActivityDetector::new();

        // Start speech
        let loud_audio = vec![0.1f32; 3200];
        vad.process(&loud_audio);
        assert!(vad.is_speaking());

        // Reset
        vad.reset();

        assert_eq!(vad.state(), VADState::Silence);
        assert!(!vad.is_speaking());
        assert_eq!(vad.speech_duration_ms(), 0);
    }

    #[test]
    fn test_compute_rms_energy() {
        // Silent audio should have near-zero energy
        let silent = vec![0.0f32; 100];
        let energy = VoiceActivityDetector::compute_rms_energy(&silent);
        assert!(energy < 0.001);

        // Full-scale sine wave should have ~0.707 RMS
        let loud: Vec<f32> = (0..100).map(|i| (i as f32 * 0.1).sin()).collect();
        let energy = VoiceActivityDetector::compute_rms_energy(&loud);
        assert!(energy > 0.5);

        // Constant value
        let constant = vec![0.5f32; 100];
        let energy = VoiceActivityDetector::compute_rms_energy(&constant);
        assert!((energy - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_is_speech_helper() {
        let silent = vec![0.001f32; 100];
        assert!(!is_speech(&silent, 0.01));

        let loud = vec![0.1f32; 100];
        assert!(is_speech(&loud, 0.01));
    }

    #[test]
    fn test_compute_energy_helper() {
        let audio = vec![0.5f32; 100];
        let energy = compute_energy(&audio);
        assert!((energy - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_vad_speech_duration() {
        let mut vad = VoiceActivityDetector::new();

        // Process 1 second of loud audio (16000 samples at 16kHz)
        let loud_audio = vec![0.1f32; 16000];
        vad.process(&loud_audio);

        let duration = vad.speech_duration_ms();
        assert!((900..=1100).contains(&duration)); // ~1000ms with some tolerance
    }

    #[test]
    fn test_vad_short_speech_filtered() {
        let config = VADConfig {
            energy_threshold: 0.01,
            silence_duration_ms: 100,
            min_speech_duration_ms: 500, // Require 500ms minimum
            frame_size: 160,
        };
        let mut vad = VoiceActivityDetector::with_config(config);

        // Very short burst of speech (< 500ms)
        let loud_audio = vec![0.1f32; 320]; // ~20ms
        vad.process(&loud_audio);

        // Follow with silence
        let silent_audio = vec![0.001f32; 3200];
        let state = vad.process(&silent_audio);

        // Should be back to silence, not speech ended (speech was too short)
        assert_eq!(state, VADState::Silence);
    }

    #[test]
    fn test_vad_empty_audio() {
        let mut vad = VoiceActivityDetector::new();

        // Empty audio should not change state
        let state = vad.process(&[]);
        assert_eq!(state, VADState::Silence);
    }

    #[test]
    fn test_vad_config_duration_to_samples() {
        let config = VADConfig::default();

        // 1000ms at 16kHz = 16000 samples
        assert_eq!(config.duration_to_samples(1000), 16000);

        // 100ms at 16kHz = 1600 samples
        assert_eq!(config.duration_to_samples(100), 1600);

        // 0ms = 0 samples
        assert_eq!(config.duration_to_samples(0), 0);
    }

    #[test]
    fn test_vad_immediate_new_speech() {
        let config = VADConfig {
            energy_threshold: 0.01,
            silence_duration_ms: 100,
            min_speech_duration_ms: 50,
            frame_size: 160,
        };
        let mut vad = VoiceActivityDetector::with_config(config);

        // Start speech
        let loud_audio = vec![0.1f32; 320];
        vad.process(&loud_audio);

        // Add enough silence to end speech
        let silent_audio = vec![0.001f32; 3200];
        vad.process(&silent_audio);
        assert!(vad.is_speech_ended());

        // New speech starts immediately
        let loud_audio = vec![0.1f32; 320];
        let state = vad.process(&loud_audio);

        assert_eq!(state, VADState::Speaking);
    }
}
