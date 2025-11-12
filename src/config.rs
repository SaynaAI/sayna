use std::env;
use std::path::PathBuf;

/// Parse a boolean value from a string, supporting multiple formats
///
/// Accepts: "true", "false", "1", "0", "yes", "no" (case insensitive)
fn parse_bool(s: &str) -> Option<bool> {
    match s.to_lowercase().as_str() {
        "true" | "1" | "yes" => Some(true),
        "false" | "0" | "no" => Some(false),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub livekit_url: String,
    pub livekit_public_url: String,
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
    pub auth_service_url: Option<String>,
    pub auth_signing_key_path: Option<PathBuf>,
    pub auth_api_secret: Option<String>,
    pub auth_timeout_seconds: u64,
    pub auth_required: bool,
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
        let livekit_public_url =
            env::var("LIVEKIT_PUBLIC_URL").unwrap_or_else(|_| "http://localhost:7880".to_string());
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
        let auth_service_url = env::var("AUTH_SERVICE_URL").ok();
        let auth_signing_key_path = env::var("AUTH_SIGNING_KEY_PATH").ok().map(PathBuf::from);
        let auth_api_secret = env::var("AUTH_API_SECRET").ok();
        let auth_timeout_seconds = env::var("AUTH_TIMEOUT_SECONDS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(5);
        let auth_required = env::var("AUTH_REQUIRED")
            .ok()
            .and_then(|v| parse_bool(&v))
            .unwrap_or(false);

        // Validate JWT auth configuration if partially provided (regardless of auth_required)
        // This ensures better error messages for misconfiguration
        if auth_service_url.is_some() || auth_signing_key_path.is_some() {
            if auth_service_url.is_none() {
                return Err(
                    "AUTH_SERVICE_URL is required when AUTH_SIGNING_KEY_PATH is set".into(),
                );
            }
            if auth_signing_key_path.is_none() {
                return Err(
                    "AUTH_SIGNING_KEY_PATH is required when AUTH_SERVICE_URL is set".into(),
                );
            }
            // Check if the signing key file exists
            if let Some(ref key_path) = auth_signing_key_path {
                if !key_path.exists() {
                    return Err(format!(
                        "AUTH_SIGNING_KEY_PATH file does not exist: {}",
                        key_path.display()
                    )
                    .into());
                }
            }
        }

        // Validate that when auth is required, at least one auth method is configured
        if auth_required {
            let has_jwt_auth = auth_service_url.is_some() && auth_signing_key_path.is_some();
            let has_api_secret = auth_api_secret.is_some();

            if !has_jwt_auth && !has_api_secret {
                return Err(
                    "When AUTH_REQUIRED=true, either (AUTH_SERVICE_URL + AUTH_SIGNING_KEY_PATH) or AUTH_API_SECRET must be configured".into()
                );
            }
        }

        Ok(ServerConfig {
            host,
            port,
            livekit_url,
            livekit_public_url,
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
            auth_service_url,
            auth_signing_key_path,
            auth_api_secret,
            auth_timeout_seconds,
            auth_required,
        })
    }

    pub fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Check if JWT-based authentication is configured
    ///
    /// Returns true if both AUTH_SERVICE_URL and AUTH_SIGNING_KEY_PATH are set
    pub fn has_jwt_auth(&self) -> bool {
        self.auth_service_url.is_some() && self.auth_signing_key_path.is_some()
    }

    /// Check if API secret authentication is configured
    ///
    /// Returns true if AUTH_API_SECRET is set
    pub fn has_api_secret_auth(&self) -> bool {
        self.auth_api_secret.is_some()
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
    use serial_test::serial;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_get_api_key_deepgram_success() {
        let config = ServerConfig {
            host: "localhost".to_string(),
            port: 3001,
            livekit_url: "ws://localhost:7880".to_string(),
            livekit_public_url: "http://localhost:7880".to_string(),
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
            auth_service_url: None,
            auth_signing_key_path: None,
            auth_api_secret: None,
            auth_timeout_seconds: 5,
            auth_required: false,
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
            livekit_public_url: "http://localhost:7880".to_string(),
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
            auth_service_url: None,
            auth_signing_key_path: None,
            auth_api_secret: None,
            auth_timeout_seconds: 5,
            auth_required: false,
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
            livekit_public_url: "http://localhost:7880".to_string(),
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
            auth_service_url: None,
            auth_signing_key_path: None,
            auth_api_secret: None,
            auth_timeout_seconds: 5,
            auth_required: false,
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
            livekit_public_url: "http://localhost:7880".to_string(),
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
            auth_service_url: None,
            auth_signing_key_path: None,
            auth_api_secret: None,
            auth_timeout_seconds: 5,
            auth_required: false,
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
            livekit_public_url: "http://localhost:7880".to_string(),
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
            auth_service_url: None,
            auth_signing_key_path: None,
            auth_api_secret: None,
            auth_timeout_seconds: 5,
            auth_required: false,
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

    #[test]
    fn test_parse_bool_true_variants() {
        assert_eq!(parse_bool("true"), Some(true));
        assert_eq!(parse_bool("TRUE"), Some(true));
        assert_eq!(parse_bool("1"), Some(true));
        assert_eq!(parse_bool("yes"), Some(true));
        assert_eq!(parse_bool("YES"), Some(true));
        assert_eq!(parse_bool("Yes"), Some(true));
    }

    #[test]
    fn test_parse_bool_false_variants() {
        assert_eq!(parse_bool("false"), Some(false));
        assert_eq!(parse_bool("FALSE"), Some(false));
        assert_eq!(parse_bool("0"), Some(false));
        assert_eq!(parse_bool("no"), Some(false));
        assert_eq!(parse_bool("NO"), Some(false));
        assert_eq!(parse_bool("No"), Some(false));
    }

    #[test]
    fn test_parse_bool_invalid() {
        assert_eq!(parse_bool("invalid"), None);
        assert_eq!(parse_bool("2"), None);
        assert_eq!(parse_bool(""), None);
        assert_eq!(parse_bool("maybe"), None);
    }

    // Helper to clean up environment variables after tests
    fn cleanup_env_vars() {
        unsafe {
            env::remove_var("AUTH_REQUIRED");
            env::remove_var("AUTH_SERVICE_URL");
            env::remove_var("AUTH_SIGNING_KEY_PATH");
            env::remove_var("AUTH_API_SECRET");
            env::remove_var("AUTH_TIMEOUT_SECONDS");
            env::remove_var("HOST");
            env::remove_var("PORT");
        }
    }

    #[test]
    #[serial]
    fn test_from_env_auth_disabled_defaults() {
        cleanup_env_vars();

        // Auth disabled by default
        let config = ServerConfig::from_env().expect("Should load config");
        assert!(!config.auth_required);
        assert_eq!(config.auth_timeout_seconds, 5);
        assert!(config.auth_service_url.is_none());
        assert!(config.auth_signing_key_path.is_none());

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_auth_required_true_variants() {
        cleanup_env_vars();
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("test_key.pem");
        fs::write(&key_path, "fake key content").unwrap();

        unsafe {
            // Test "true"
            env::set_var("AUTH_REQUIRED", "true");
            env::set_var("AUTH_SERVICE_URL", "http://auth.example.com");
            env::set_var("AUTH_SIGNING_KEY_PATH", key_path.to_str().unwrap());
        }
        let config = ServerConfig::from_env().expect("Should load config");
        assert!(config.auth_required);

        unsafe {
            // Test "1"
            env::set_var("AUTH_REQUIRED", "1");
        }
        let config = ServerConfig::from_env().expect("Should load config");
        assert!(config.auth_required);

        unsafe {
            // Test "yes"
            env::set_var("AUTH_REQUIRED", "yes");
        }
        let config = ServerConfig::from_env().expect("Should load config");
        assert!(config.auth_required);

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_auth_required_false_variants() {
        cleanup_env_vars();

        unsafe {
            // Test "false"
            env::set_var("AUTH_REQUIRED", "false");
        }
        let config = ServerConfig::from_env().expect("Should load config");
        assert!(!config.auth_required);

        unsafe {
            // Test "0"
            env::set_var("AUTH_REQUIRED", "0");
        }
        let config = ServerConfig::from_env().expect("Should load config");
        assert!(!config.auth_required);

        unsafe {
            // Test "no"
            env::set_var("AUTH_REQUIRED", "no");
        }
        let config = ServerConfig::from_env().expect("Should load config");
        assert!(!config.auth_required);

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_auth_service_url() {
        cleanup_env_vars();
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("test_key.pem");
        fs::write(&key_path, "fake key content").unwrap();

        unsafe {
            env::set_var("AUTH_REQUIRED", "true");
            env::set_var("AUTH_SERVICE_URL", "https://auth.service.example.com");
            env::set_var("AUTH_SIGNING_KEY_PATH", key_path.to_str().unwrap());
        }

        let config = ServerConfig::from_env().expect("Should load config");
        assert_eq!(
            config.auth_service_url,
            Some("https://auth.service.example.com".to_string())
        );

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_auth_signing_key_path() {
        cleanup_env_vars();
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("signing_key.pem");
        fs::write(&key_path, "fake key content").unwrap();

        unsafe {
            env::set_var("AUTH_REQUIRED", "true");
            env::set_var("AUTH_SERVICE_URL", "http://auth.example.com");
            env::set_var("AUTH_SIGNING_KEY_PATH", key_path.to_str().unwrap());
        }

        let config = ServerConfig::from_env().expect("Should load config");
        assert_eq!(config.auth_signing_key_path, Some(key_path));

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_auth_timeout_default() {
        cleanup_env_vars();

        let config = ServerConfig::from_env().expect("Should load config");
        assert_eq!(config.auth_timeout_seconds, 5);

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_auth_timeout_custom() {
        cleanup_env_vars();
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("test_key.pem");
        fs::write(&key_path, "fake key content").unwrap();

        unsafe {
            env::set_var("AUTH_REQUIRED", "true");
            env::set_var("AUTH_SERVICE_URL", "http://auth.example.com");
            env::set_var("AUTH_SIGNING_KEY_PATH", key_path.to_str().unwrap());
            env::set_var("AUTH_TIMEOUT_SECONDS", "10");
        }

        let config = ServerConfig::from_env().expect("Should load config");
        assert_eq!(config.auth_timeout_seconds, 10);

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_auth_required_missing_url() {
        cleanup_env_vars();
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("test_key.pem");
        fs::write(&key_path, "fake key content").unwrap();

        unsafe {
            env::set_var("AUTH_REQUIRED", "true");
            env::set_var("AUTH_SIGNING_KEY_PATH", key_path.to_str().unwrap());
            // Missing AUTH_SERVICE_URL (and no AUTH_API_SECRET either)
        }

        let result = ServerConfig::from_env();
        assert!(result.is_err());
        // Should fail because when AUTH_SIGNING_KEY_PATH is set, AUTH_SERVICE_URL is required
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("AUTH_SERVICE_URL is required")
        );

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_auth_required_missing_key_path() {
        cleanup_env_vars();

        unsafe {
            env::set_var("AUTH_REQUIRED", "true");
            env::set_var("AUTH_SERVICE_URL", "http://auth.example.com");
            // Missing AUTH_SIGNING_KEY_PATH (and no AUTH_API_SECRET either)
        }

        let result = ServerConfig::from_env();
        assert!(result.is_err());
        // Should fail because when AUTH_SERVICE_URL is set, AUTH_SIGNING_KEY_PATH is required
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("AUTH_SIGNING_KEY_PATH is required")
        );

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_auth_required_key_file_not_exists() {
        cleanup_env_vars();

        unsafe {
            env::set_var("AUTH_REQUIRED", "true");
            env::set_var("AUTH_SERVICE_URL", "http://auth.example.com");
            env::set_var("AUTH_SIGNING_KEY_PATH", "/nonexistent/key.pem");
        }

        let result = ServerConfig::from_env();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("file does not exist")
        );

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_host_and_port() {
        cleanup_env_vars();

        unsafe {
            env::set_var("HOST", "127.0.0.1");
            env::set_var("PORT", "8080");
        }

        let config = ServerConfig::from_env().expect("Should load config");
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 8080);

        cleanup_env_vars();
    }

    // API Secret authentication tests

    #[test]
    #[serial]
    fn test_from_env_api_secret_auth_enabled() {
        cleanup_env_vars();

        unsafe {
            env::set_var("AUTH_REQUIRED", "true");
            env::set_var("AUTH_API_SECRET", "my-super-secret-token");
        }

        let config = ServerConfig::from_env().expect("Should load config");
        assert!(config.auth_required);
        assert_eq!(
            config.auth_api_secret,
            Some("my-super-secret-token".to_string())
        );
        assert!(config.auth_service_url.is_none());
        assert!(config.auth_signing_key_path.is_none());

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_api_secret_missing_when_required() {
        cleanup_env_vars();

        unsafe {
            env::set_var("AUTH_REQUIRED", "true");
            // No AUTH_API_SECRET or AUTH_SERVICE_URL
        }

        let result = ServerConfig::from_env();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("either (AUTH_SERVICE_URL + AUTH_SIGNING_KEY_PATH) or AUTH_API_SECRET"));

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_env_both_auth_methods_configured() {
        cleanup_env_vars();
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("test_key.pem");
        fs::write(&key_path, "fake key content").unwrap();

        unsafe {
            env::set_var("AUTH_REQUIRED", "true");
            env::set_var("AUTH_SERVICE_URL", "http://auth.example.com");
            env::set_var("AUTH_SIGNING_KEY_PATH", key_path.to_str().unwrap());
            env::set_var("AUTH_API_SECRET", "my-secret");
        }

        // Both auth methods configured should be valid
        let config = ServerConfig::from_env().expect("Should load config with both auth methods");
        assert!(config.auth_required);
        assert_eq!(config.auth_api_secret, Some("my-secret".to_string()));
        assert_eq!(
            config.auth_service_url,
            Some("http://auth.example.com".to_string())
        );
        assert_eq!(config.auth_signing_key_path, Some(key_path));

        cleanup_env_vars();
    }

    #[test]
    fn test_has_jwt_auth() {
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("test_key.pem");

        let config_with_jwt = ServerConfig {
            host: "localhost".to_string(),
            port: 3001,
            livekit_url: "ws://localhost:7880".to_string(),
            livekit_public_url: "http://localhost:7880".to_string(),
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
            auth_service_url: Some("http://auth.example.com".to_string()),
            auth_signing_key_path: Some(key_path),
            auth_api_secret: None,
            auth_timeout_seconds: 5,
            auth_required: true,
        };

        assert!(config_with_jwt.has_jwt_auth());
        assert!(!config_with_jwt.has_api_secret_auth());

        let config_without_jwt = ServerConfig {
            host: "localhost".to_string(),
            port: 3001,
            livekit_url: "ws://localhost:7880".to_string(),
            livekit_public_url: "http://localhost:7880".to_string(),
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
            auth_service_url: None,
            auth_signing_key_path: None,
            auth_api_secret: None,
            auth_timeout_seconds: 5,
            auth_required: false,
        };

        assert!(!config_without_jwt.has_jwt_auth());
    }

    #[test]
    fn test_has_api_secret_auth() {
        let config_with_api_secret = ServerConfig {
            host: "localhost".to_string(),
            port: 3001,
            livekit_url: "ws://localhost:7880".to_string(),
            livekit_public_url: "http://localhost:7880".to_string(),
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
            auth_service_url: None,
            auth_signing_key_path: None,
            auth_api_secret: Some("my-secret-token".to_string()),
            auth_timeout_seconds: 5,
            auth_required: true,
        };

        assert!(config_with_api_secret.has_api_secret_auth());
        assert!(!config_with_api_secret.has_jwt_auth());

        let config_without_api_secret = ServerConfig {
            host: "localhost".to_string(),
            port: 3001,
            livekit_url: "ws://localhost:7880".to_string(),
            livekit_public_url: "http://localhost:7880".to_string(),
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
            auth_service_url: None,
            auth_signing_key_path: None,
            auth_api_secret: None,
            auth_timeout_seconds: 5,
            auth_required: false,
        };

        assert!(!config_without_api_secret.has_api_secret_auth());
    }
}
