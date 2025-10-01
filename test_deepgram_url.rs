// Quick test to verify Deepgram URL formatting
use sayna::core::stt::deepgram::{DeepgramSTT, DeepgramSTTConfig};
use sayna::core::stt::base::STTConfig;

fn main() {
    let config = DeepgramSTTConfig {
        base: STTConfig {
            model: "nova-3".to_string(),
            provider: "deepgram".to_string(),
            api_key: "test-key".to_string(),
            language: "en-US".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
        },
        diarize: false,
        interim_results: true,
        filler_words: false,
        profanity_filter: false,
        smart_format: true,
        keywords: vec!["hello".to_string(), "world".to_string()],
        redact: Vec::new(),
        vad_events: true,
        endpointing: Some(200),
        tag: Some("test-tag".to_string()),
        utterance_end_ms: Some(500),
    };

    let stt = DeepgramSTT::default();
    match stt.build_websocket_url(&config) {
        Ok(url) => {
            println!("Generated URL:");
            println!("{}", url);
            println!("\nURL Parameters:");
            for param in url.split('&') {
                if param.contains('?') {
                    let parts: Vec<&str> = param.split('?').collect();
                    println!("  {}", parts[1]);
                } else {
                    println!("  {}", param);
                }
            }
        }
        Err(e) => eprintln!("Error building URL: {}", e),
    }
}