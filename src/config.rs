use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub livekit_url: String,
    pub livekit_api_key: Option<String>,
    pub livekit_api_secret: Option<String>,

    pub deepgram_api_key: Option<String>,
    pub elevenlabs_api_key: Option<String>,

    // LiveKit recording configuration
    pub recording_s3_bucket: Option<String>,
    pub recording_s3_region: Option<String>,
    pub recording_s3_endpoint: Option<String>,
    pub recording_s3_access_key: Option<String>,
    pub recording_s3_secret_key: Option<String>,

    // Cache configuration (filesystem or memory)
    pub cache_path: Option<PathBuf>, // if None, use in-memory cache
    pub cache_ttl_seconds: Option<u64>,

    // Authentication configuration
    pub auth_decryption_key: String, // if empty, auth is disabled
}

impl ServerConfig {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        // Load .env file if it exists
        let _ = dotenvy::dotenv();

        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port = env::var("PORT")
            .unwrap_or_else(|_| "3001".to_string())
            .parse::<u16>()
            .map_err(|e| format!("Invalid port number: {e}"))?;

        let livekit_url =
            env::var("LIVEKIT_URL").unwrap_or_else(|_| "ws://localhost:7880".to_string());
        let livekit_api_key = env::var("LIVEKIT_API_KEY").ok();
        let livekit_api_secret = env::var("LIVEKIT_API_SECRET").ok();

        let deepgram_api_key = env::var("DEEPGRAM_API_KEY").ok();
        let elevenlabs_api_key = env::var("ELEVENLABS_API_KEY").ok();

        // LiveKit recording S3 configuration from env
        let recording_s3_bucket = env::var("RECORDING_S3_BUCKET").ok();
        let recording_s3_region = env::var("RECORDING_S3_REGION").ok();
        let recording_s3_endpoint = env::var("RECORDING_S3_ENDPOINT").ok();
        let recording_s3_access_key = env::var("RECORDING_S3_ACCESS_KEY").ok();
        let recording_s3_secret_key = env::var("RECORDING_S3_SECRET_KEY").ok();

        // Cache configuration from env
        let cache_path = env::var("CACHE_PATH").ok().map(PathBuf::from);
        let cache_ttl_seconds = env::var("CACHE_TTL_SECONDS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .or(Some(30 * 24 * 60 * 60)); // 1 month (30 days) default

        // Authentication configuration from env
        let auth_decryption_key = env::var("AUTH_DECRYPTION_KEY").unwrap_or_else(|_| String::new());

        Ok(ServerConfig {
            host,
            port,
            livekit_url,
            livekit_api_key,
            livekit_api_secret,
            deepgram_api_key,
            elevenlabs_api_key,
            recording_s3_bucket,
            recording_s3_region,
            recording_s3_endpoint,
            recording_s3_access_key,
            recording_s3_secret_key,
            cache_path,
            cache_ttl_seconds,
            auth_decryption_key,
        })
    }

    pub fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Get API key for a specific provider
    ///
    /// # Arguments
    /// * `provider` - The name of the provider (e.g., "deepgram", "elevenlabs")
    ///
    /// # Returns
    /// * `Result<String, String>` - The API key on success, or an error message on failure
    ///
    /// # Example
    /// ```rust,no_run
    /// use sayna::config::ServerConfig;
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = ServerConfig::from_env()?;
    /// let api_key = config.get_api_key("deepgram")?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn get_api_key(&self, provider: &str) -> Result<String, String> {
        match provider.to_lowercase().as_str() {
            "deepgram" => {
                self.deepgram_api_key.as_ref().cloned().ok_or_else(|| {
                    "Deepgram API key not configured in server environment".to_string()
                })
            }
            "elevenlabs" => self.elevenlabs_api_key.as_ref().cloned().ok_or_else(|| {
                "ElevenLabs API key not configured in server environment".to_string()
            }),
            _ => Err(format!("Unsupported provider: {provider}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_api_key_deepgram_success() {
        let config = ServerConfig {
            host: "localhost".to_string(),
            port: 3001,
            livekit_url: "ws://localhost:7880".to_string(),
            livekit_api_key: None,
            livekit_api_secret: None,
            deepgram_api_key: Some("test-deepgram-key".to_string()),
            elevenlabs_api_key: None,
            recording_s3_bucket: None,
            recording_s3_region: None,
            recording_s3_endpoint: None,
            recording_s3_access_key: None,
            recording_s3_secret_key: None,
            cache_path: None,
            cache_ttl_seconds: Some(3600),
            auth_decryption_key: String::new(),
        };

        let result = config.get_api_key("deepgram");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "test-deepgram-key");
    }

    #[test]
    fn test_get_api_key_elevenlabs_success() {
        let config = ServerConfig {
            host: "localhost".to_string(),
            port: 3001,
            livekit_url: "ws://localhost:7880".to_string(),
            livekit_api_key: None,
            livekit_api_secret: None,
            deepgram_api_key: None,
            elevenlabs_api_key: Some("test-elevenlabs-key".to_string()),
            recording_s3_bucket: None,
            recording_s3_region: None,
            recording_s3_endpoint: None,
            recording_s3_access_key: None,
            recording_s3_secret_key: None,
            cache_path: None,
            cache_ttl_seconds: Some(3600),
            auth_decryption_key: String::new(),
        };

        let result = config.get_api_key("elevenlabs");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "test-elevenlabs-key");
    }

    #[test]
    fn test_get_api_key_deepgram_missing() {
        let config = ServerConfig {
            host: "localhost".to_string(),
            port: 3001,
            livekit_url: "ws://localhost:7880".to_string(),
            livekit_api_key: None,
            livekit_api_secret: None,
            deepgram_api_key: None,
            elevenlabs_api_key: None,
            recording_s3_bucket: None,
            recording_s3_region: None,
            recording_s3_endpoint: None,
            recording_s3_access_key: None,
            recording_s3_secret_key: None,
            cache_path: None,
            cache_ttl_seconds: Some(3600),
            auth_decryption_key: String::new(),
        };

        let result = config.get_api_key("deepgram");
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "Deepgram API key not configured in server environment"
        );
    }

    #[test]
    fn test_get_api_key_unsupported_provider() {
        let config = ServerConfig {
            host: "localhost".to_string(),
            port: 3001,
            livekit_url: "ws://localhost:7880".to_string(),
            livekit_api_key: None,
            livekit_api_secret: None,
            deepgram_api_key: Some("test-key".to_string()),
            elevenlabs_api_key: None,
            recording_s3_bucket: None,
            recording_s3_region: None,
            recording_s3_endpoint: None,
            recording_s3_access_key: None,
            recording_s3_secret_key: None,
            cache_path: None,
            cache_ttl_seconds: Some(3600),
            auth_decryption_key: String::new(),
        };

        let result = config.get_api_key("unsupported_provider");
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "Unsupported provider: unsupported_provider"
        );
    }

    #[test]
    fn test_get_api_key_case_insensitive() {
        let config = ServerConfig {
            host: "localhost".to_string(),
            port: 3001,
            livekit_url: "ws://localhost:7880".to_string(),
            livekit_api_key: None,
            livekit_api_secret: None,
            deepgram_api_key: Some("test-deepgram-key".to_string()),
            elevenlabs_api_key: Some("test-elevenlabs-key".to_string()),
            recording_s3_bucket: None,
            recording_s3_region: None,
            recording_s3_endpoint: None,
            recording_s3_access_key: None,
            recording_s3_secret_key: None,
            cache_path: None,
            cache_ttl_seconds: Some(3600),
            auth_decryption_key: String::new(),
        };

        // Test uppercase
        let result1 = config.get_api_key("DEEPGRAM");
        assert!(result1.is_ok());
        assert_eq!(result1.unwrap(), "test-deepgram-key");

        // Test mixed case
        let result2 = config.get_api_key("ElevenLabs");
        assert!(result2.is_ok());
        assert_eq!(result2.unwrap(), "test-elevenlabs-key");
    }
}
