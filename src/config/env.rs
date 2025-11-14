use std::env;
use std::path::PathBuf;

use super::ServerConfig;
use super::utils::parse_bool;
use super::validation::{validate_auth_required, validate_jwt_auth};

impl ServerConfig {
    /// Load configuration from environment variables
    ///
    /// Reads configuration from environment variables, with sensible defaults.
    /// Also loads from .env file if present using dotenvy.
    ///
    /// # Returns
    /// * `Result<Self, Box<dyn std::error::Error>>` - The loaded configuration or an error
    ///
    /// # Errors
    /// Returns an error if:
    /// - Required environment variables are malformed
    /// - Authentication configuration is invalid
    /// - JWT signing key file doesn't exist
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        // Load .env file if it exists
        let _ = dotenvy::dotenv();

        // Server configuration
        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port = env::var("PORT")
            .unwrap_or_else(|_| "3001".to_string())
            .parse::<u16>()
            .map_err(|e| format!("Invalid port number: {e}"))?;

        // LiveKit configuration
        let livekit_url =
            env::var("LIVEKIT_URL").unwrap_or_else(|_| "ws://localhost:7880".to_string());
        let livekit_public_url =
            env::var("LIVEKIT_PUBLIC_URL").unwrap_or_else(|_| "http://localhost:7880".to_string());
        let livekit_api_key = env::var("LIVEKIT_API_KEY").ok();
        let livekit_api_secret = env::var("LIVEKIT_API_SECRET").ok();

        // Provider API keys
        let deepgram_api_key = env::var("DEEPGRAM_API_KEY").ok();
        let elevenlabs_api_key = env::var("ELEVENLABS_API_KEY").ok();

        // LiveKit recording S3 configuration
        let recording_s3_bucket = env::var("RECORDING_S3_BUCKET").ok();
        let recording_s3_region = env::var("RECORDING_S3_REGION").ok();
        let recording_s3_endpoint = env::var("RECORDING_S3_ENDPOINT").ok();
        let recording_s3_access_key = env::var("RECORDING_S3_ACCESS_KEY").ok();
        let recording_s3_secret_key = env::var("RECORDING_S3_SECRET_KEY").ok();

        // Cache configuration
        let cache_path = env::var("CACHE_PATH").ok().map(PathBuf::from);
        let cache_ttl_seconds = env::var("CACHE_TTL_SECONDS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .or(Some(30 * 24 * 60 * 60)); // 1 month (30 days) default

        // Authentication configuration
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

        // Validate JWT auth configuration if partially provided
        validate_jwt_auth(&auth_service_url, &auth_signing_key_path)?;

        // Validate that when auth is required, at least one auth method is configured
        validate_auth_required(
            auth_required,
            &auth_service_url,
            &auth_signing_key_path,
            &auth_api_secret,
        )?;

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::fs;
    use tempfile::TempDir;

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
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("either (AUTH_SERVICE_URL + AUTH_SIGNING_KEY_PATH) or AUTH_API_SECRET")
        );

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
}
