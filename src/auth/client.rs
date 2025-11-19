use crate::auth::jwt::{AuthPayload, load_private_key, sign_auth_request_with_key};
use crate::config::ServerConfig;
use crate::errors::auth_error::{AuthError, AuthResult};
use jsonwebtoken::{Algorithm, EncodingKey};
use reqwest::{Client, StatusCode};
use std::collections::HashMap;
use std::time::Duration;

/// HTTP client for communicating with the external authentication service
#[derive(Clone)]
pub struct AuthClient {
    /// HTTP client for making requests
    client: Client,
    /// URL of the authentication service
    auth_service_url: String,
    /// Preloaded encoding key for signing JWTs
    encoding_key: EncodingKey,
    /// Algorithm to use for JWT signing (RS256 or ES256)
    algorithm: Algorithm,
}

impl std::fmt::Debug for AuthClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthClient")
            .field("auth_service_url", &self.auth_service_url)
            .field("algorithm", &self.algorithm)
            .field("encoding_key", &"<redacted>")
            .finish()
    }
}

impl AuthClient {
    /// Create a new AuthClient from server configuration (async version with preloaded key)
    ///
    /// This async constructor preloads the private signing key into memory,
    /// avoiding repeated disk I/O on every authentication request.
    ///
    /// # Arguments
    /// * `config` - Server configuration containing auth settings
    ///
    /// # Returns
    /// * `AuthResult<Self>` - New AuthClient or ConfigError if auth is not configured
    pub async fn from_config(config: &ServerConfig) -> AuthResult<Self> {
        let auth_service_url = config.auth_service_url.as_ref().ok_or_else(|| {
            AuthError::ConfigError("AUTH_SERVICE_URL is not configured".to_string())
        })?;

        let signing_key_path = config.auth_signing_key_path.as_ref().ok_or_else(|| {
            AuthError::ConfigError("AUTH_SIGNING_KEY_PATH is not configured".to_string())
        })?;

        // Preload the signing key and algorithm (cached, single disk read)
        let (encoding_key, algorithm) = load_private_key(signing_key_path)?;

        let timeout = Duration::from_secs(config.auth_timeout_seconds);

        let client = Client::builder()
            .timeout(timeout)
            .pool_max_idle_per_host(10) // Connection pooling
            .build()
            .map_err(|e| AuthError::ConfigError(format!("Failed to create HTTP client: {e}")))?;

        Ok(Self {
            client,
            auth_service_url: auth_service_url.clone(),
            encoding_key,
            algorithm,
        })
    }

    /// Validate a bearer token by calling the external auth service
    ///
    /// # Arguments
    /// * `token` - The bearer token to validate
    /// * `request_body` - The original request body as JSON
    /// * `request_headers` - Filtered request headers to include in validation
    /// * `request_path` - The request path (e.g., "/speak")
    /// * `request_method` - The HTTP method (e.g., "POST")
    ///
    /// # Returns
    /// * `AuthResult<()>` - Ok if token is valid, AuthError otherwise
    ///
    /// # Example
    /// ```rust,no_run
    /// use sayna::auth::AuthClient;
    /// use sayna::config::ServerConfig;
    /// use std::collections::HashMap;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = ServerConfig::from_env()?;
    /// let client = AuthClient::from_config(&config).await?;
    ///
    /// client.validate_token(
    ///     "user-bearer-token",
    ///     &serde_json::json!({"text": "Hello"}),
    ///     HashMap::new(),
    ///     "/speak",
    ///     "POST",
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn validate_token(
        &self,
        token: &str,
        request_body: &serde_json::Value,
        request_headers: HashMap<String, String>,
        request_path: &str,
        request_method: &str,
    ) -> AuthResult<()> {
        // Create the auth payload
        let payload = AuthPayload {
            token: token.to_string(),
            request_body: request_body.clone(),
            request_headers,
            request_path: request_path.to_string(),
            request_method: request_method.to_string(),
        };

        // Sign the payload with our preloaded private key
        let jwt = sign_auth_request_with_key(&payload, &self.encoding_key, self.algorithm)?;

        // Send the JWT to the auth service
        let response = self
            .client
            .post(&self.auth_service_url)
            .header("Content-Type", "application/jwt")
            .body(jwt)
            .send()
            .await?; // Use ? to leverage the From<reqwest::Error> conversion

        // Check the response status
        let status = response.status();
        match status {
            StatusCode::OK => {
                tracing::debug!("Token validation successful");
                Ok(())
            }
            _ => {
                // Cap error body at 500 characters to prevent DoS
                const MAX_ERROR_BODY_LEN: usize = 500;
                let error_body = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Unable to read error body".to_string());

                let capped_body = if error_body.len() > MAX_ERROR_BODY_LEN {
                    format!("{}... (truncated)", &error_body[..MAX_ERROR_BODY_LEN])
                } else {
                    error_body
                };

                if status == StatusCode::UNAUTHORIZED {
                    Err(AuthError::Unauthorized(capped_body))
                } else {
                    // All other statuses (including 4xx and 5xx) go through AuthServiceError
                    // which will be mapped to appropriate HTTP status by the error handler
                    Err(AuthError::AuthServiceError(status, capped_body))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method},
    };

    // Test RSA private key (2048-bit, generated for testing only, DO NOT USE IN PRODUCTION)
    const TEST_RSA_PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQCMrnQRHYGWk8lv
FGwO/Dr6HVPxIc5FONC8Bz2E24pmUIEgRhvOaUsdlak1PZNiJU23eLcsa8/UtlGx
B56i0PNXliiEpU9tCS5sCKf9PTwD5PkJowep4xBdcQMK1MdcDMMULpfJYc08o9rE
zGHtoX4mnT2yk+/hAn9wCdvH0hceNFgxUhXtiJBToIjiC+ANqd3IjlCmcCZH1DhT
Uwgzern6nmo0GetFydjeXFuPUiZdIprY2yRZtZ20j2PkowBroIGIiXTr7/X6fIRA
9Jg4rrSDdoTsv15KBNcpcwoRqJ55/r4mevz3tWea2Mv1hxyJ5ywGnqoMC5X5G3Ur
aoAriQHXAgMBAAECggEACD3qN9x6LJew6+yO3hvh2qhgNBbObljDRdjIvmFcTN03
i2wAEgoyJ+QOOzvFyDC2SmLsnFIepXAe/hebsB88umtmKUtECXfJu/OP3/K38uR1
wJ5IAyh123uU+Yv4uAhZX3PRWa98pipVVUVCEXluGhYJOM6Y9Z4/WBGDykOhLhg9
tr3zmHJFGp0OXmmT4b/WTlwF6hvafaDb9mjAsL0fhK1r9+/U75q0QltJiLlIg5Gb
q0kmQcG60/5Z2NZEizlH1lDB+gCC2s5H9CxXorLsZEpEDcHKkMT5sCM5tic5EMH2
HNXXG+tC3ph/XvmK7Ow+rJEi3mMXwZFs95NJXQBBBQKBgQDGFrklHAUEQZL4Izgh
GIDeL2caUSenXMMmpnD4kLs6P9F8UeQfErLZ2J3D5HayuAKAnj0wiwLOlrGTxC9N
RaVsrVVidTn599b1ICD+fPidpfXhaye3wezrzVeVf3IOYa2LVHsiZZ9nVsHEbBKz
i0yYw3aIlv/PTsSWtf8JumUVowKBgQC1z0bOuIUCuiGApTo00V/NR1IQ8+tX+yU7
dn61SoyKtgmHA1Po+WgEDWi4zehFWU6xUog/KCTzm0VWTGIjh70Rv9w1N7QX/iug
O8bpX8uekZUEt6ShddJYsyhd+e5WwWdxgjm7UOLXgOggJFjpx8CBzlIj9LtbJAJ9
LeS/zItePQKBgQC3EEzuZKSmOEuwkivPOivuKfSot5Nj8jBPycXhkS/WNyBMOgoO
RWOQO8YhQUQJClEVuCdocy+W6GEX5FiqmtC0TMP6B8gaoNbBFn4ncir41mUTe8nq
4ocnrE9i07L+Y3rUprBdK3lTMTRFaHMoBnY1P36N4K5sUakQdwVJYj8E7QKBgQCA
P8T9EeCR+eakLumOVJu13LehSc8b8wdimMXs8LePKbYyzUAlubmMEkFrC6TrNoJy
R3vgwVq/lSomJB+eXKQcnzChQbgCrMLtdv1rpq2mH5/1Ae5aDxjghRDWqfVcsXVc
9rXu0rIRvtb/xWQLFWNQrc/3mS2IrzAqSXNxcMJnKQKBgBiooJ/PPRuwg9ZFJYoc
Sn8fw7fRDvRsslnHus1GgXZYWBbMwbNxGGO0Gy68i2SDu30gPlSVLzb+a8GD6qD/
8unmgG3V7uLXrpNIu5to6SggvL2mt5F036GFEnR2pnPH08ooqQc4u7u2k9P79pS8
V/reoL3Jcy/mQ9MrmJx+K1VC
-----END PRIVATE KEY-----"#;

    /// Helper to create a test RSA private key from bundled fixture
    fn create_test_key() -> (TempDir, PathBuf) {
        use std::io::Write;

        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("test_key.pem");

        let mut file = std::fs::File::create(&key_path).unwrap();
        file.write_all(TEST_RSA_PRIVATE_KEY.as_bytes()).unwrap();
        file.flush().unwrap();

        (temp_dir, key_path)
    }

    #[tokio::test]
    async fn test_from_config_missing_url() {
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
            auth_api_secret: None,
            auth_service_url: None,
            auth_signing_key_path: Some(PathBuf::from("/tmp/key.pem")),
            auth_timeout_seconds: 5,
            auth_required: false,
            sip: None,
        };

        let result = AuthClient::from_config(&config).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AuthError::ConfigError(_)));
    }

    #[tokio::test]
    async fn test_from_config_missing_key_path() {
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
            auth_api_secret: None,
            auth_service_url: Some("http://auth.example.com".to_string()),
            auth_signing_key_path: None,
            auth_timeout_seconds: 5,
            auth_required: false,
            sip: None,
        };

        let result = AuthClient::from_config(&config).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AuthError::ConfigError(_)));
    }

    #[tokio::test]
    async fn test_validate_token_success_200() {
        let (_temp_dir, key_path) = create_test_key();

        // Start a mock server
        let mock_server = MockServer::start().await;

        // Mock a 200 OK response
        Mock::given(method("POST"))
            .and(header("content-type", "application/jwt"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&mock_server)
            .await;

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
            auth_api_secret: None,
            auth_service_url: Some(mock_server.uri()),
            auth_signing_key_path: Some(key_path),
            auth_timeout_seconds: 5,
            auth_required: true,
            sip: None,
        };

        let client = AuthClient::from_config(&config).await.unwrap();

        let result = client
            .validate_token(
                "test-token",
                &serde_json::json!({"test": "data"}),
                HashMap::new(),
                "/test",
                "POST",
            )
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_validate_token_unauthorized_401() {
        let (_temp_dir, key_path) = create_test_key();

        let mock_server = MockServer::start().await;

        // Mock a 401 Unauthorized response
        Mock::given(method("POST"))
            .and(header("content-type", "application/jwt"))
            .respond_with(ResponseTemplate::new(401).set_body_string("Invalid token"))
            .expect(1)
            .mount(&mock_server)
            .await;

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
            auth_api_secret: None,
            auth_service_url: Some(mock_server.uri()),
            auth_signing_key_path: Some(key_path),
            auth_timeout_seconds: 5,
            auth_required: true,
            sip: None,
        };

        let client = AuthClient::from_config(&config).await.unwrap();

        let result = client
            .validate_token(
                "invalid-token",
                &serde_json::json!({}),
                HashMap::new(),
                "/test",
                "POST",
            )
            .await;

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AuthError::Unauthorized(_)));
    }

    #[tokio::test]
    async fn test_validate_token_server_error_500() {
        let (_temp_dir, key_path) = create_test_key();

        let mock_server = MockServer::start().await;

        // Mock a 500 Internal Server Error response
        Mock::given(method("POST"))
            .and(header("content-type", "application/jwt"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Internal server error"))
            .expect(1)
            .mount(&mock_server)
            .await;

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
            auth_api_secret: None,
            auth_service_url: Some(mock_server.uri()),
            auth_signing_key_path: Some(key_path),
            auth_timeout_seconds: 5,
            auth_required: true,
            sip: None,
        };

        let client = AuthClient::from_config(&config).await.unwrap();

        let result = client
            .validate_token(
                "test-token",
                &serde_json::json!({}),
                HashMap::new(),
                "/test",
                "POST",
            )
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        // 5xx should be mapped to AuthServiceError
        assert!(matches!(err, AuthError::AuthServiceError(_, _)));
    }

    #[tokio::test]
    async fn test_validate_token_timeout() {
        let (_temp_dir, key_path) = create_test_key();

        let mock_server = MockServer::start().await;

        // Mock a delayed response that exceeds timeout
        Mock::given(method("POST"))
            .and(header("content-type", "application/jwt"))
            .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(10)))
            .expect(1)
            .mount(&mock_server)
            .await;

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
            auth_api_secret: None,
            auth_service_url: Some(mock_server.uri()),
            auth_signing_key_path: Some(key_path),
            auth_timeout_seconds: 1, // 1 second timeout
            auth_required: true,
            sip: None,
        };

        let client = AuthClient::from_config(&config).await.unwrap();

        let result = client
            .validate_token(
                "test-token",
                &serde_json::json!({}),
                HashMap::new(),
                "/test",
                "POST",
            )
            .await;

        assert!(result.is_err());
        // Timeout should be converted to HttpError via From<reqwest::Error>
        assert!(matches!(result.unwrap_err(), AuthError::HttpError(_)));
    }

    #[tokio::test]
    async fn test_validate_token_error_body_capping() {
        let (_temp_dir, key_path) = create_test_key();

        let mock_server = MockServer::start().await;

        // Create a large error body (> 500 chars)
        let large_error = "X".repeat(1000);

        Mock::given(method("POST"))
            .and(header("content-type", "application/jwt"))
            .respond_with(ResponseTemplate::new(401).set_body_string(large_error))
            .expect(1)
            .mount(&mock_server)
            .await;

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
            auth_api_secret: None,
            auth_service_url: Some(mock_server.uri()),
            auth_signing_key_path: Some(key_path),
            auth_timeout_seconds: 5,
            auth_required: true,
            sip: None,
        };

        let client = AuthClient::from_config(&config).await.unwrap();

        let result = client
            .validate_token(
                "test-token",
                &serde_json::json!({}),
                HashMap::new(),
                "/test",
                "POST",
            )
            .await;

        assert!(result.is_err());
        if let Err(AuthError::Unauthorized(msg)) = result {
            // Verify the error message was capped and includes truncation notice
            assert!(msg.len() <= 515); // 500 + "... (truncated)"
            assert!(msg.contains("(truncated)"));
        } else {
            panic!("Expected Unauthorized error");
        }
    }
}
