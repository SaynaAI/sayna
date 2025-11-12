//! Configuration module for Sayna server
//!
//! This module handles server configuration from various sources: YAML files and
//! environment variables. Environment variables always override YAML values.
//! The configuration is split into logical submodules for maintainability and extensibility.
//!
//! # Modules
//! - `yaml`: YAML configuration file loading
//! - `env`: Environment variable loading
//! - `merge`: Merging YAML and environment configurations
//! - `validation`: Configuration validation logic
//! - `utils`: Utility functions for configuration parsing
//!
//! # Example
//! ```rust,no_run
//! use sayna::config::ServerConfig;
//! use std::path::PathBuf;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Load from environment variables only
//! let config = ServerConfig::from_env()?;
//!
//! // Load from YAML file with environment variable overrides
//! let config_path = PathBuf::from("config.yaml");
//! let config = ServerConfig::from_file(&config_path)?;
//!
//! println!("Server listening on {}", config.address());
//! # Ok(())
//! # }
//! ```

use std::path::PathBuf;

mod env;
mod merge;
mod utils;
mod validation;
mod yaml;

/// Server configuration
///
/// Contains all configuration needed to run the Sayna server, including:
/// - Server settings (host, port)
/// - LiveKit integration settings
/// - Provider API keys (Deepgram, ElevenLabs)
/// - Recording configuration (S3)
/// - Cache settings
/// - Authentication settings
#[derive(Debug, Clone)]
pub struct ServerConfig {
    // Server settings
    pub host: String,
    pub port: u16,

    // LiveKit settings
    pub livekit_url: String,
    pub livekit_public_url: String,
    pub livekit_api_key: Option<String>,
    pub livekit_api_secret: Option<String>,

    // Provider API keys
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
    /// Load configuration from a YAML file with environment variable overrides
    ///
    /// Loads configuration from the specified YAML file, then applies environment
    /// variable overrides. This allows YAML to provide base configuration while
    /// environment variables can override specific values.
    ///
    /// Priority order (highest to lowest):
    /// 1. Environment variables
    /// 2. YAML file values
    /// 3. Default values
    ///
    /// After loading and merging, performs validation on the final configuration.
    ///
    /// # Arguments
    /// * `path` - Path to the YAML configuration file
    ///
    /// # Returns
    /// * `Result<Self, Box<dyn std::error::Error>>` - The loaded configuration or an error
    ///
    /// # Errors
    /// Returns an error if:
    /// - The YAML file cannot be read or is malformed
    /// - Environment variables have invalid formats
    /// - Configuration validation fails
    ///
    /// # Example
    /// ```rust,no_run
    /// use sayna::config::ServerConfig;
    /// use std::path::PathBuf;
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let config_path = PathBuf::from("config.yaml");
    /// let config = ServerConfig::from_file(&config_path)?;
    /// println!("Server listening on {}", config.address());
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_file(path: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        // Note: We do NOT load .env file here since the user explicitly specified a YAML config file.
        // If they want .env values, they should either:
        // 1. Set actual environment variables, or
        // 2. Include values in the YAML file
        // This gives more predictable behavior - YAML file is the source of truth, and
        // only actual environment variables (not .env) override it.

        // Load YAML configuration
        let yaml_config = yaml::YamlConfig::from_file(path)?;

        // Merge with environment variables
        let config = merge::merge_config(Some(yaml_config))?;

        // Validate configuration
        validation::validate_jwt_auth(&config.auth_service_url, &config.auth_signing_key_path)?;
        validation::validate_auth_required(
            config.auth_required,
            &config.auth_service_url,
            &config.auth_signing_key_path,
            &config.auth_api_secret,
        )?;

        Ok(config)
    }

    /// Get the server address as a string
    ///
    /// Returns the address in the format "host:port"
    ///
    /// # Example
    /// ```rust,no_run
    /// use sayna::config::ServerConfig;
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = ServerConfig::from_env()?;
    /// println!("Listening on {}", config.address());
    /// # Ok(())
    /// # }
    /// ```
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
            "deepgram" => self.deepgram_api_key.as_ref().cloned().ok_or_else(|| {
                "Deepgram API key not configured in server environment".to_string()
            }),
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
    use std::env;
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

    // Helper to clean up environment variables
    fn cleanup_env_vars() {
        unsafe {
            env::remove_var("HOST");
            env::remove_var("PORT");
            env::remove_var("LIVEKIT_URL");
            env::remove_var("LIVEKIT_PUBLIC_URL");
            env::remove_var("DEEPGRAM_API_KEY");
            env::remove_var("ELEVENLABS_API_KEY");
            env::remove_var("CACHE_PATH");
            env::remove_var("CACHE_TTL_SECONDS");
            env::remove_var("AUTH_REQUIRED");
            env::remove_var("AUTH_SERVICE_URL");
            env::remove_var("AUTH_SIGNING_KEY_PATH");
            env::remove_var("AUTH_API_SECRET");
            env::remove_var("AUTH_TIMEOUT_SECONDS");
        }
    }

    #[test]
    #[serial]
    fn test_from_file_yaml_only() {
        cleanup_env_vars();

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yaml");

        let yaml_content = r#"
server:
  host: "127.0.0.1"
  port: 8080

providers:
  deepgram_api_key: "yaml-dg-key"
  elevenlabs_api_key: "yaml-el-key"

cache:
  path: "/tmp/yaml-cache"
  ttl_seconds: 7200
"#;

        fs::write(&config_path, yaml_content).unwrap();

        let config = ServerConfig::from_file(&config_path).unwrap();

        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 8080);
        assert_eq!(config.deepgram_api_key, Some("yaml-dg-key".to_string()));
        assert_eq!(config.elevenlabs_api_key, Some("yaml-el-key".to_string()));
        assert_eq!(config.cache_path, Some(PathBuf::from("/tmp/yaml-cache")));
        assert_eq!(config.cache_ttl_seconds, Some(7200));

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_file_env_overrides_yaml() {
        cleanup_env_vars();

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yaml");

        let yaml_content = r#"
server:
  host: "127.0.0.1"
  port: 8080

providers:
  deepgram_api_key: "yaml-key"
"#;

        fs::write(&config_path, yaml_content).unwrap();

        unsafe {
            env::set_var("HOST", "0.0.0.0");
            env::set_var("DEEPGRAM_API_KEY", "env-key");
        }

        let config = ServerConfig::from_file(&config_path).unwrap();

        // ENV overrides YAML
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.deepgram_api_key, Some("env-key".to_string()));
        // YAML value used when no ENV
        assert_eq!(config.port, 8080);

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_file_missing_file() {
        cleanup_env_vars();

        let config_path = PathBuf::from("/nonexistent/config.yaml");
        let result = ServerConfig::from_file(&config_path);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to read config file"));

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_file_invalid_yaml() {
        cleanup_env_vars();

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("invalid.yaml");

        fs::write(&config_path, "invalid: yaml: [content").unwrap();

        let result = ServerConfig::from_file(&config_path);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to parse YAML"));

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_file_with_auth() {
        cleanup_env_vars();

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yaml");
        let key_path = temp_dir.path().join("key.pem");
        fs::write(&key_path, "fake key").unwrap();

        let yaml_content = format!(
            r#"
auth:
  required: true
  service_url: "https://auth.example.com"
  signing_key_path: "{}"
  timeout_seconds: 10
"#,
            key_path.display()
        );

        fs::write(&config_path, yaml_content).unwrap();

        let config = ServerConfig::from_file(&config_path).unwrap();

        assert!(config.auth_required);
        assert_eq!(
            config.auth_service_url,
            Some("https://auth.example.com".to_string())
        );
        assert_eq!(config.auth_signing_key_path, Some(key_path));
        assert_eq!(config.auth_timeout_seconds, 10);

        cleanup_env_vars();
    }

    #[test]
    #[serial]
    fn test_from_file_partial_config() {
        cleanup_env_vars();

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yaml");

        let yaml_content = r#"
server:
  port: 9000

cache:
  ttl_seconds: 1800
"#;

        fs::write(&config_path, yaml_content).unwrap();

        let config = ServerConfig::from_file(&config_path).unwrap();

        // YAML values
        assert_eq!(config.port, 9000);
        assert_eq!(config.cache_ttl_seconds, Some(1800));

        // Default values
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.livekit_url, "ws://localhost:7880");
        assert!(!config.auth_required);

        cleanup_env_vars();
    }
}
