use std::time::Duration;

use bytes::Bytes;
use google_api_proto::google::cloud::speech::v2::{
    ExplicitDecodingConfig, RecognitionConfig, RecognitionFeatures, StreamingRecognitionConfig,
    StreamingRecognitionFeatures, StreamingRecognizeRequest, StreamingRecognizeResponse,
    explicit_decoding_config::AudioEncoding, streaming_recognition_features::VoiceActivityTimeout,
    streaming_recognize_request::StreamingRequest, streaming_recognize_response::SpeechEventType,
};
use tokio::sync::mpsc;
use tokio::time::Instant;
use tracing::{debug, error, info, warn};

use crate::core::stt::base::{STTError, STTResult};

use super::config::GoogleSTTConfig;

/// Maximum audio chunk size (25KB) as recommended by Google for optimal streaming.
/// Smaller chunks reduce latency but increase overhead; larger chunks increase latency.
pub(super) const MAX_AUDIO_CHUNK_SIZE: usize = 25 * 1024;

/// Keep-alive interval in seconds. Google's server-side timeout is ~10 seconds,
/// so we send keep-alive audio every second to prevent timeout.
pub(super) const KEEPALIVE_INTERVAL_SECS: u64 = 1;

/// Duration of silence to send as keep-alive (20ms of audio at 16kHz, 16-bit mono = 640 bytes).
/// This is short enough to not interfere with speech detection but keeps the stream alive.
pub(super) const KEEPALIVE_SILENCE_DURATION_MS: u64 = 20;

/// Generate silent audio bytes for keep-alive.
/// The size is calculated based on sample rate, channels, and bytes per sample.
#[inline]
pub(super) fn generate_silence_audio(sample_rate: u32, channels: u32, duration_ms: u64) -> Bytes {
    // For LINEAR16, each sample is 2 bytes
    let bytes_per_sample = 2u32;
    let num_samples = (sample_rate as u64 * duration_ms / 1000) as usize;
    let total_bytes = num_samples * channels as usize * bytes_per_sample as usize;

    // Pre-allocate and fill with zeros (silence for LINEAR16)
    Bytes::from(vec![0u8; total_bytes])
}

/// Tracks the last activity time for keep-alive logic.
#[derive(Debug, Clone)]
pub(super) struct KeepaliveTracker {
    pub last_activity: Instant,
    pub sample_rate: u32,
    pub channels: u32,
}

impl KeepaliveTracker {
    pub fn new(sample_rate: u32, channels: u32) -> Self {
        Self {
            last_activity: Instant::now(),
            sample_rate,
            channels,
        }
    }

    /// Update the last activity timestamp.
    #[inline]
    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    /// Check if keep-alive is needed (no activity for KEEPALIVE_INTERVAL_SECS).
    #[inline]
    pub fn needs_keepalive(&self) -> bool {
        self.last_activity.elapsed() >= Duration::from_secs(KEEPALIVE_INTERVAL_SECS)
    }

    /// Generate keep-alive silence audio.
    #[inline]
    pub fn generate_keepalive(&self) -> Bytes {
        generate_silence_audio(
            self.sample_rate,
            self.channels,
            KEEPALIVE_SILENCE_DURATION_MS,
        )
    }
}

pub(super) fn map_encoding_to_proto(encoding: &str) -> AudioEncoding {
    match encoding.to_lowercase().as_str() {
        "linear16" | "pcm" => AudioEncoding::Linear16,
        "mulaw" | "ulaw" => AudioEncoding::Mulaw,
        "alaw" => AudioEncoding::Alaw,
        _ => AudioEncoding::Linear16,
    }
}

pub(super) fn to_prost_duration(duration: Duration) -> prost_types::Duration {
    prost_types::Duration {
        seconds: duration.as_secs() as i64,
        nanos: duration.subsec_nanos() as i32,
    }
}

pub(super) fn build_config_request(config: &GoogleSTTConfig) -> StreamingRecognizeRequest {
    let decoding_config = Some(
        google_api_proto::google::cloud::speech::v2::recognition_config::DecodingConfig::ExplicitDecodingConfig(
            ExplicitDecodingConfig {
                encoding: map_encoding_to_proto(&config.base.encoding) as i32,
                sample_rate_hertz: config.base.sample_rate as i32,
                audio_channel_count: config.base.channels as i32,
            }
        )
    );

    let features = Some(RecognitionFeatures {
        enable_automatic_punctuation: config.base.punctuation,
        ..Default::default()
    });

    let recognition_config = Some(RecognitionConfig {
        decoding_config,
        model: config.base.model.clone(),
        language_codes: vec![config.base.language.clone()],
        features,
        ..Default::default()
    });

    let voice_activity_timeout =
        if config.speech_start_timeout.is_some() || config.speech_end_timeout.is_some() {
            Some(VoiceActivityTimeout {
                speech_start_timeout: config.speech_start_timeout.map(to_prost_duration),
                speech_end_timeout: config.speech_end_timeout.map(to_prost_duration),
            })
        } else {
            None
        };

    let streaming_features = Some(StreamingRecognitionFeatures {
        interim_results: config.interim_results,
        enable_voice_activity_events: config.enable_voice_activity_events,
        voice_activity_timeout,
    });

    let streaming_config = StreamingRecognitionConfig {
        config: recognition_config,
        config_mask: None,
        streaming_features,
    };

    StreamingRecognizeRequest {
        recognizer: config.recognizer_path(),
        streaming_request: Some(StreamingRequest::StreamingConfig(streaming_config)),
    }
}

/// Build an audio request for streaming recognition.
/// Uses Bytes for zero-copy data transfer on the hot path.
/// Takes owned String to avoid allocation on each call - caller should clone once if needed.
#[inline]
pub(super) fn build_audio_request(
    audio_data: Bytes,
    recognizer: String,
) -> StreamingRecognizeRequest {
    StreamingRecognizeRequest {
        recognizer,
        streaming_request: Some(StreamingRequest::Audio(audio_data)),
    }
}

/// Chunk audio data into pieces suitable for streaming.
/// Returns an iterator to avoid allocating a Vec when not needed.
/// Uses Bytes::slice for zero-copy chunking.
#[inline]
pub(super) fn chunk_audio(audio_data: Bytes) -> impl Iterator<Item = Bytes> {
    ChunkIterator {
        data: audio_data,
        offset: 0,
    }
}

/// Iterator that yields audio chunks without unnecessary allocations.
struct ChunkIterator {
    data: Bytes,
    offset: usize,
}

impl Iterator for ChunkIterator {
    type Item = Bytes;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.offset >= self.data.len() {
            return None;
        }

        let end = (self.offset + MAX_AUDIO_CHUNK_SIZE).min(self.data.len());
        let chunk = self.data.slice(self.offset..end);
        self.offset = end;
        Some(chunk)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.data.len().saturating_sub(self.offset);
        let chunks = remaining.div_ceil(MAX_AUDIO_CHUNK_SIZE);
        (chunks, Some(chunks))
    }
}

impl ExactSizeIterator for ChunkIterator {}

/// Legacy function for backward compatibility with tests.
/// Converts Vec<u8> to Bytes chunks.
#[cfg(test)]
pub(super) fn chunk_audio_vec(audio_data: Vec<u8>) -> Vec<Vec<u8>> {
    let bytes = Bytes::from(audio_data);
    chunk_audio(bytes).map(|b| b.to_vec()).collect()
}

/// Determine if this event marks the end of speech.
/// Called on every streaming response - inlined for performance.
#[inline]
pub(super) fn determine_speech_final(event_type: SpeechEventType, is_final: bool) -> bool {
    match event_type {
        SpeechEventType::EndOfSingleUtterance => true,
        SpeechEventType::SpeechActivityEnd => is_final,
        _ => is_final,
    }
}

/// Get confidence score, returning 0.0 for interim results.
/// Called on every result - inlined for performance.
#[inline]
pub(super) fn get_confidence(confidence: f32, is_final: bool) -> f32 {
    if is_final && confidence > 0.0 {
        confidence
    } else {
        0.0
    }
}

pub(super) fn handle_grpc_error(status: tonic::Status) -> STTError {
    match status.code() {
        tonic::Code::Unauthenticated => {
            error!(
                code = ?status.code(),
                message = %status.message(),
                "Authentication failed - check credentials"
            );
            STTError::AuthenticationFailed(status.message().to_string())
        }
        tonic::Code::PermissionDenied => {
            error!(
                code = ?status.code(),
                message = %status.message(),
                "Permission denied - check API permissions"
            );
            STTError::AuthenticationFailed(format!("Permission denied: {}", status.message()))
        }
        tonic::Code::InvalidArgument => {
            error!(
                code = ?status.code(),
                message = %status.message(),
                "Invalid argument in request"
            );
            STTError::ConfigurationError(status.message().to_string())
        }
        tonic::Code::ResourceExhausted => {
            warn!(
                code = ?status.code(),
                message = %status.message(),
                "Rate limit exceeded"
            );
            STTError::ProviderError(format!("Rate limit exceeded: {}", status.message()))
        }
        tonic::Code::Unavailable => {
            warn!(
                code = ?status.code(),
                message = %status.message(),
                "Service unavailable"
            );
            STTError::NetworkError(format!("Service unavailable: {}", status.message()))
        }
        tonic::Code::DeadlineExceeded => {
            warn!(
                code = ?status.code(),
                message = %status.message(),
                "Request timeout"
            );
            STTError::NetworkError(format!("Request timeout: {}", status.message()))
        }
        tonic::Code::Cancelled => {
            debug!(
                code = ?status.code(),
                message = %status.message(),
                "Request cancelled"
            );
            STTError::ProviderError(format!("Request cancelled: {}", status.message()))
        }
        tonic::Code::Internal => {
            error!(
                code = ?status.code(),
                message = %status.message(),
                "Internal server error"
            );
            STTError::ProviderError(format!("Internal error: {}", status.message()))
        }
        _ => {
            error!(
                code = ?status.code(),
                message = %status.message(),
                "gRPC error occurred"
            );
            STTError::ProviderError(format!(
                "gRPC error {}: {}",
                status.code(),
                status.message()
            ))
        }
    }
}

pub(super) fn handle_streaming_response(
    response: StreamingRecognizeResponse,
    result_tx: &mpsc::UnboundedSender<STTResult>,
) -> Result<(), STTError> {
    let event_type = SpeechEventType::try_from(response.speech_event_type)
        .unwrap_or(SpeechEventType::Unspecified);

    debug!(
        event_type = ?event_type,
        results_count = response.results.len(),
        "Received streaming response"
    );

    match event_type {
        SpeechEventType::SpeechActivityBegin => {
            debug!(
                offset = ?response.speech_event_offset,
                "Speech activity began - voice detected"
            );
        }
        SpeechEventType::SpeechActivityEnd => {
            debug!(
                offset = ?response.speech_event_offset,
                "Speech activity ended - voice stopped"
            );
        }
        SpeechEventType::EndOfSingleUtterance => {
            info!(
                offset = ?response.speech_event_offset,
                "End of single utterance detected - finalizing transcription"
            );
        }
        SpeechEventType::Unspecified => {}
    }

    for (result_idx, result) in response.results.iter().enumerate() {
        if result.alternatives.is_empty() {
            debug!(result_idx, "Skipping result with no alternatives");
            continue;
        }

        let top_alt = &result.alternatives[0];

        debug!(
            result_idx,
            is_final = result.is_final,
            stability = result.stability,
            confidence = top_alt.confidence,
            transcript_len = top_alt.transcript.len(),
            language_code = %result.language_code,
            "Processing recognition result"
        );

        if top_alt.transcript.is_empty() && !result.is_final {
            debug!(result_idx, "Skipping interim result with empty transcript");
            continue;
        }

        let stt_result = STTResult::new(
            top_alt.transcript.clone(),
            result.is_final,
            determine_speech_final(event_type, result.is_final),
            get_confidence(top_alt.confidence, result.is_final),
        );

        debug!(
            transcript = %stt_result.transcript,
            is_final = stt_result.is_final,
            is_speech_final = stt_result.is_speech_final,
            confidence = stt_result.confidence,
            "Sending transcription result"
        );

        if result_tx.send(stt_result).is_err() {
            warn!("Failed to send STT result - channel closed");
            return Err(STTError::ProviderError("Result channel closed".to_string()));
        }
    }

    Ok(())
}
