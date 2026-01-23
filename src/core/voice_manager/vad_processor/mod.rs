//! VAD-based audio processing for silence detection.
//!
//! Feature-gated under `stt-vad`. VAD event handling is in `STTResultProcessor`.

mod processor;
mod state;

#[cfg(test)]
mod tests;

pub use processor::process_audio_chunk;
pub use state::VADState;
