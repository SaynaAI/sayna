mod base;

// Re-export public types and traits
pub use base::{
    BaseSTT, STTConfig, STTConnectionState, STTError, STTFactory, STTHelper, STTResult,
    STTResultCallback, STTStats,
};

/// Example usage of the STT trait abstraction
///
/// This demonstrates how to create a custom STT provider implementation
/// and use it with the unified interface.
///
/// ```rust
/// use sayna::core::stt::{BaseSTT, STTConfig, STTResult, STTResultCallback};
/// use std::sync::Arc;
/// use std::pin::Pin;
/// use std::future::Future;
///
/// // Usage example:
/// async fn example_usage() {
///     let mut stt_provider = create_stt_provider();
///     
///     // Configure the provider
///     let config = STTConfig {
///         api_key: "your-api-key".to_string(),
///         language: "en-US".to_string(),
///         sample_rate: 16000,
///         channels: 1,
///         punctuation: true,
///     };
///     
///     // Connect to the provider
///     stt_provider.connect(config).await.unwrap();
///     
///     // Register a callback for results
///     let callback = Arc::new(|result: STTResult| {
///         Box::pin(async move {
///             println!("Transcription: {}", result.transcript);
///             println!("Final: {}, Confidence: {:.2}", result.is_final, result.confidence);
///         }) as Pin<Box<dyn Future<Output = ()> + Send>>
///     });
///     
///     stt_provider.on_result(callback).await.unwrap();
///     
///     // Send audio data
///     let audio_data = vec![0u8; 1024]; // Your audio bytes here
///     stt_provider.send_audio(audio_data).await.unwrap();
///     
///     // Disconnect when done
///     stt_provider.disconnect().await.unwrap();
/// }
/// ```
#[cfg(doc)]
pub mod example {
    use super::*;

    /// Example implementation showing how to create a custom STT provider
    pub struct ExampleSTTProvider {
        // Implementation details would go here
    }

    /// Factory function to create STT providers
    pub fn create_stt_provider() -> Box<dyn BaseSTT> {
        // This would return an actual provider implementation
        // For example: Box::new(DeepgramSTT::new()) or Box::new(GoogleSTT::new())
        todo!("Implement actual STT provider")
    }
}
