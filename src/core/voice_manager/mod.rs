//! # Voice Manager
//!
//! This module provides a unified interface for managing both Speech-to-Text (STT) and
//! Text-to-Speech (TTS) providers in a single, coordinated system. The VoiceManager
//! abstracts away the complexity of managing multiple providers and provides a clean
//! API for real-time voice processing.
//!
//! ## Features
//!
//! - **Unified Management**: Coordinate STT and TTS providers through a single interface
//! - **Real-time Processing**: Optimized for low-latency voice processing
//! - **Speech Final Timing Control**: Automatic fallback mechanism for delayed `is_speech_final` signals in noisy environments
//! - **Error Handling**: Comprehensive error handling with proper error propagation
//! - **Callback System**: Event-driven architecture for handling results
//! - **Thread Safety**: Safe concurrent access using Arc<RwLock<>>
//! - **Provider Abstraction**: Support for multiple STT and TTS providers
//!
//! ## Usage Example
//!
//! ```rust,no_run
//! use sayna::core::voice_manager::{VoiceManager, VoiceManagerConfig};
//! use sayna::core::stt::STTConfig;
//! use sayna::core::tts::TTSConfig;
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Configure STT and TTS providers
//!     let config = VoiceManagerConfig {
//!         stt_config: STTConfig {
//!             provider: "deepgram".to_string(),
//!             api_key: "your-stt-api-key".to_string(),
//!             language: "en-US".to_string(),
//!             sample_rate: 16000,
//!             channels: 1,
//!             punctuation: true,
//!             encoding: "linear16".to_string(),
//!             model: "nova-3".to_string(),
//!         },
//!         tts_config: TTSConfig {
//!             provider: "deepgram".to_string(),
//!             api_key: "your-tts-api-key".to_string(),
//!             voice_id: Some("aura-luna-en".to_string()),
//!             speaking_rate: Some(1.0),
//!             audio_format: Some("pcm".to_string()),
//!             sample_rate: Some(22050),
//!             ..Default::default()
//!         },
//!     };
//!
//!     // Create and start the voice manager
//!     let voice_manager = VoiceManager::new(config, None)?;
//!     voice_manager.start().await?;
//!
//!     // Wait for both providers to be ready
//!     while !voice_manager.is_ready().await {
//!         tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
//!     }
//!
//!     // Register callbacks for STT results
//!     voice_manager.on_stt_result(|result| {
//!         Box::pin(async move {
//!             println!("STT Result: {} (confidence: {:.2})", result.transcript, result.confidence);
//!             if result.is_final {
//!                 println!("Final transcription: {}", result.transcript);
//!             }
//!         })
//!     }).await?;
//!
//!     // Register callbacks for TTS audio
//!     voice_manager.on_tts_audio(|audio_data| {
//!         Box::pin(async move {
//!             println!("Received {} bytes of audio in {} format",
//!                      audio_data.data.len(), audio_data.format);
//!             // Process audio data here (e.g., play through speakers)
//!         })
//!     }).await?;
//!
//!     // Register callbacks for TTS errors
//!     voice_manager.on_tts_error(|error| {
//!         Box::pin(async move {
//!             eprintln!("TTS Error: {}", error);
//!         })
//!     }).await?;
//!
//!     // Example: Send audio for transcription
//!     let audio_data = vec![0u8; 1024]; // Your audio data here
//!     voice_manager.receive_audio(audio_data).await?;
//!
//!     // Example: Synthesize speech
//!     voice_manager.speak("Hello, this is a test message", true).await?;
//!     voice_manager.flush_tts().await?; // Ensure immediate processing
//!
//!     // Example: Multiple speech requests
//!     voice_manager.speak("First message.", false).await?;
//!     voice_manager.speak("Second message.", false).await?;
//!     voice_manager.speak("Third message.", true).await?;
//!
//!     // Clear any pending TTS requests
//!     voice_manager.clear_tts().await?;
//!
//!     // Check individual provider status
//!     println!("STT ready: {}", voice_manager.is_stt_ready().await);
//!     println!("TTS ready: {}", voice_manager.is_tts_ready().await);
//!
//!     // Get provider information
//!     println!("STT provider: {}", voice_manager.get_stt_provider_info().await);
//!     println!("TTS provider: {}", voice_manager.get_tts_provider_info().await);
//!
//!     // Stop the voice manager
//!     voice_manager.stop().await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Advanced Usage
//!
//! ### Real-time Voice Processing
//!
//! ```rust,no_run
//! use sayna::core::voice_manager::{VoiceManager, VoiceManagerConfig};
//! use sayna::core::stt::STTConfig;
//! use sayna::core::tts::TTSConfig;
//! use tokio::sync::mpsc;
//! use std::sync::Arc;
//!
//! async fn realtime_voice_processing() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = VoiceManagerConfig {
//!         stt_config: STTConfig {
//!             provider: "deepgram".to_string(),
//!             api_key: "your-stt-api-key".to_string(),
//!             ..Default::default()
//!         },
//!         tts_config: TTSConfig {
//!             provider: "deepgram".to_string(),
//!             api_key: "your-tts-api-key".to_string(),
//!             ..Default::default()
//!         },
//!     };
//!     let voice_manager = Arc::new(VoiceManager::new(config, None)?);
//!     let vm = voice_manager.clone();
//!     
//!     // Start the voice manager
//!     vm.start().await?;
//!
//!     // Channel for audio input
//!     let (audio_tx, mut audio_rx) = mpsc::unbounded_channel::<Vec<u8>>();
//!
//!     // Set up STT callback for real-time transcription
//!     let vm_clone = vm.clone();
//!     vm.on_stt_result(move |result| {
//!         let vm = vm_clone.clone();
//!         Box::pin(async move {
//!             if result.is_final && !result.transcript.trim().is_empty() {
//!                 // Echo the transcription back as speech
//!                 let response = format!("You said: {}", result.transcript);
//!                 if let Err(e) = vm.speak(&response, true).await {
//!                     eprintln!("Failed to speak response: {}", e);
//!                 }
//!             }
//!         })
//!     }).await?;
//!
//!     // Audio processing loop
//!     tokio::spawn(async move {
//!         while let Some(audio_data) = audio_rx.recv().await {
//!             if let Err(e) = voice_manager.receive_audio(audio_data).await {
//!                 eprintln!("Failed to process audio: {}", e);
//!             }
//!         }
//!     });
//!
//!     // Simulate audio input (in real app, this would come from microphone)
//!     for i in 0..10 {
//!         let audio_chunk = vec![0u8; 1024]; // Mock audio data
//!         audio_tx.send(audio_chunk)?;
//!         tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! ### Error Handling and Recovery
//!
//! ```rust,no_run
//! use sayna::core::voice_manager::{VoiceManager, VoiceManagerConfig};
//! use sayna::core::stt::STTConfig;
//! use sayna::core::tts::TTSConfig;
//!
//! async fn robust_voice_processing() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = VoiceManagerConfig {
//!         stt_config: STTConfig {
//!             provider: "deepgram".to_string(),
//!             api_key: "your-stt-api-key".to_string(),
//!             ..Default::default()
//!         },
//!         tts_config: TTSConfig {
//!             provider: "deepgram".to_string(),
//!             api_key: "your-tts-api-key".to_string(),
//!             ..Default::default()
//!         },
//!     };
//!     let voice_manager = VoiceManager::new(config, None)?;
//!     
//!     // Start with error handling
//!     match voice_manager.start().await {
//!         Ok(_) => println!("Voice manager started successfully"),
//!         Err(e) => {
//!             eprintln!("Failed to start voice manager: {}", e);
//!             return Err(e.into());
//!         }
//!     }
//!
//!     // Set up error callback for TTS
//!     voice_manager.on_tts_error(|error| {
//!         Box::pin(async move {
//!             eprintln!("TTS Error occurred: {}", error);
//!             // Implement recovery logic here
//!         })
//!     }).await?;
//!
//!     // Check provider readiness with timeout
//!     let timeout = tokio::time::Duration::from_secs(30);
//!     let start_time = tokio::time::Instant::now();
//!     
//!     while !voice_manager.is_ready().await {
//!         if start_time.elapsed() > timeout {
//!             return Err("Timeout waiting for providers to be ready".into());
//!         }
//!         tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
//!     }
//!
//!     // Process with error recovery
//!     match voice_manager.speak("Test message", true).await {
//!         Ok(_) => println!("Speech synthesis successful"),
//!         Err(e) => {
//!             eprintln!("Speech synthesis failed: {}", e);
//!             // Implement retry logic or fallback
//!         }
//!     }
//!
//!     Ok(())
//! }
//! ```

pub mod callbacks;
pub mod config;
pub mod errors;
pub mod manager;
pub mod state;
pub mod stt_result;

#[cfg(test)]
mod tests;

// Re-export commonly used items
pub use callbacks::{AudioClearCallback, STTCallback, TTSAudioCallback, TTSErrorCallback};
pub use config::VoiceManagerConfig;
pub use errors::{VoiceManagerError, VoiceManagerResult};
pub use manager::VoiceManager;
