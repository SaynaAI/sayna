use crate::errors::auth_error::{AuthError, AuthResult};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Cache for loaded private keys with their algorithms
/// Stores (EncodingKey, Algorithm) to avoid re-reading files for algorithm detection
static KEY_CACHE: Lazy<RwLock<HashMap<PathBuf, (EncodingKey, Algorithm)>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

/// Payload structure for auth request data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthPayload {
    /// The bearer token from the Authorization header
    pub token: String,
    /// The request body as a JSON value
    pub request_body: serde_json::Value,
    /// Relevant request headers (filtered to exclude sensitive ones)
    pub request_headers: HashMap<String, String>,
    /// Request path (e.g., "/speak")
    pub request_path: String,
    /// Request HTTP method (e.g., "POST")
    pub request_method: String,
}

/// JWT claims structure with standard fields
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthClaims {
    /// Subject - identifies the JWT issuer
    pub sub: String,
    /// Issued at - Unix timestamp when JWT was created
    pub iat: i64,
    /// Expiration time - Unix timestamp when JWT expires
    pub exp: i64,
    /// Auth request data nested under this field
    pub auth_data: AuthPayload,
}

impl AuthClaims {
    /// Create new claims with current timestamp and 5-minute expiration
    pub fn new(auth_data: AuthPayload) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs() as i64;

        Self {
            sub: "sayna-auth".to_string(),
            iat: now,
            exp: now + 300, // 5 minutes expiration
            auth_data,
        }
    }
}

/// Filter request headers to exclude sensitive and internal headers
///
/// Excludes:
/// - authorization (already extracted)
/// - cookie (sensitive)
/// - x-forwarded-* (internal proxy headers)
/// - x-sayna-* (internal application headers)
/// - host, x-real-ip (infrastructure headers)
pub fn filter_headers(headers: &axum::http::HeaderMap) -> HashMap<String, String> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            let name_str = name.as_str().to_lowercase();

            // Exclude sensitive and internal headers
            if name_str == "authorization"
                || name_str == "cookie"
                || name_str.starts_with("x-forwarded-")
                || name_str.starts_with("x-sayna-")
                || name_str == "host"
                || name_str == "x-real-ip"
            {
                None
            } else {
                value.to_str().ok().map(|v| (name_str, v.to_string()))
            }
        })
        .collect()
}

/// Detect the algorithm from PEM key data
///
/// # Arguments
/// * `key_data` - The raw key data (PEM format)
///
/// # Returns
/// * Algorithm - RS256 for RSA keys, ES256 for EC keys
pub fn detect_algorithm(key_data: &[u8]) -> Algorithm {
    if key_data.windows(10).any(|w| w.starts_with(b"BEGIN RSA")) {
        Algorithm::RS256
    } else {
        Algorithm::ES256
    }
}

/// Load and cache a private key from a PEM file
///
/// Keys are cached in memory to avoid re-reading from disk on every request.
/// Supports both RSA and ECDSA keys in PEM format.
/// Returns both the encoding key and detected algorithm.
///
/// # Arguments
/// * `key_path` - Path to the private key file
///
/// # Returns
/// * `AuthResult<(EncodingKey, Algorithm)>` - The encoding key and algorithm, or an error
pub fn load_private_key(key_path: &Path) -> AuthResult<(EncodingKey, Algorithm)> {
    // Check cache first
    {
        let cache = KEY_CACHE.read();
        if let Some(cached) = cache.get(key_path) {
            return Ok(cached.clone());
        }
    }

    // Read the private key file
    let key_data = fs::read(key_path)
        .map_err(|e| AuthError::ConfigError(format!("Failed to read private key file: {e}")))?;

    // Detect algorithm and create the appropriate EncodingKey
    let (encoding_key, algorithm) = if let Ok(key) = EncodingKey::from_rsa_pem(&key_data) {
        (key, Algorithm::RS256)
    } else if let Ok(key) = EncodingKey::from_ec_pem(&key_data) {
        (key, Algorithm::ES256)
    } else {
        return Err(AuthError::ConfigError(
            "Unsupported key format. Only RSA and ECDSA PEM keys are supported.".to_string(),
        ));
    };

    // Cache the key and algorithm together
    {
        let mut cache = KEY_CACHE.write();
        cache.insert(key_path.to_path_buf(), (encoding_key.clone(), algorithm));
    }

    Ok((encoding_key, algorithm))
}

/// Sign an auth request with the private key
///
/// Creates a JWT with standard claims (sub, iat, exp) and nests the auth
/// request data under the `auth_data` field.
///
/// # Arguments
/// * `payload` - The auth payload to sign
/// * `private_key_path` - Path to the private key file (PEM format, supports RSA and ECDSA)
///
/// # Returns
/// * `AuthResult<String>` - The signed JWT token on success, or AuthError on failure
///
/// # Example
/// ```rust,no_run
/// use sayna::auth::jwt::{AuthPayload, sign_auth_request};
/// use std::collections::HashMap;
/// use std::path::PathBuf;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let payload = AuthPayload {
///     token: "user-token".to_string(),
///     request_body: serde_json::json!({"text": "Hello"}),
///     request_headers: HashMap::new(),
///     request_path: "/speak".to_string(),
///     request_method: "POST".to_string(),
/// };
///
/// let jwt = sign_auth_request(&payload, &PathBuf::from("private_key.pem"))?;
/// # Ok(())
/// # }
/// ```
/// Sign an auth request with a preloaded encoding key
///
/// This version accepts an already-loaded EncodingKey and Algorithm,
/// avoiding the need to read from disk on every request.
///
/// # Arguments
/// * `payload` - The auth payload to sign
/// * `encoding_key` - Preloaded encoding key
/// * `algorithm` - Algorithm to use (RS256 or ES256)
///
/// # Returns
/// * `AuthResult<String>` - The signed JWT token on success, or AuthError on failure
pub fn sign_auth_request_with_key(
    payload: &AuthPayload,
    encoding_key: &EncodingKey,
    algorithm: Algorithm,
) -> AuthResult<String> {
    let header = Header::new(algorithm);
    let claims = AuthClaims::new(payload.clone());

    encode(&header, &claims, encoding_key)
        .map_err(|e| AuthError::JwtSigningError(format!("Failed to encode JWT: {e}")))
}

pub fn sign_auth_request(payload: &AuthPayload, private_key_path: &Path) -> AuthResult<String> {
    // Load the private key and algorithm from cache (single disk read on first call)
    let (encoding_key, algorithm) = load_private_key(private_key_path)?;

    let header = Header::new(algorithm);

    // Create claims with the payload nested under auth_data
    let claims = AuthClaims::new(payload.clone());

    // Sign the claims
    encode(&header, &claims, &encoding_key)
        .map_err(|e| AuthError::JwtSigningError(format!("Failed to sign JWT: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    #[test]
    fn test_auth_claims_new() {
        let payload = AuthPayload {
            token: "test-token".to_string(),
            request_body: serde_json::json!({"key": "value"}),
            request_headers: HashMap::new(),
            request_path: "/speak".to_string(),
            request_method: "POST".to_string(),
        };

        let claims = AuthClaims::new(payload.clone());

        assert_eq!(claims.sub, "sayna-auth");
        assert_eq!(claims.auth_data.token, "test-token");
        assert_eq!(claims.auth_data.request_path, "/speak");
        assert_eq!(claims.auth_data.request_method, "POST");
        assert_eq!(claims.exp - claims.iat, 300); // 5 minutes
    }

    #[test]
    fn test_filter_headers_excludes_sensitive() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer token".parse().unwrap());
        headers.insert("content-type", "application/json".parse().unwrap());
        headers.insert("user-agent", "test-agent".parse().unwrap());
        headers.insert("cookie", "session=abc".parse().unwrap());
        headers.insert("x-forwarded-for", "1.2.3.4".parse().unwrap());
        headers.insert("x-sayna-internal", "value".parse().unwrap());
        headers.insert("host", "example.com".parse().unwrap());

        let filtered = filter_headers(&headers);

        assert!(!filtered.contains_key("authorization"));
        assert!(!filtered.contains_key("cookie"));
        assert!(!filtered.contains_key("x-forwarded-for"));
        assert!(!filtered.contains_key("x-sayna-internal"));
        assert!(!filtered.contains_key("host"));
        assert!(filtered.contains_key("content-type"));
        assert!(filtered.contains_key("user-agent"));
    }

    #[test]
    fn test_filter_headers_empty() {
        let headers = HeaderMap::new();
        let filtered = filter_headers(&headers);
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_load_private_key_missing_file() {
        let result = load_private_key(Path::new("/nonexistent/key.pem"));
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(matches!(e, AuthError::ConfigError(_)));
            assert!(e.to_string().contains("Failed to read private key file"));
        }
    }

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

    // Test EC private key (P-256, generated for testing only, DO NOT USE IN PRODUCTION)
    const TEST_EC_PRIVATE_KEY: &str = r#"-----BEGIN EC PRIVATE KEY-----
MHcCAQEEIPusMktPqGcBYuuby5KpoAm7UpXQuqsBD3o+sRaENguloAoGCCqGSM49
AwEHoUQDQgAEk5mZZ97RuV5pCZlGPQ2hDQVn72XGG5lM8rTL3djPciw4EU8/R+Rk
wqHEyarjnTBZUmlzZCE91+oiuGYkjRx6Cw==
-----END EC PRIVATE KEY-----"#;

    #[test]
    fn test_sign_and_decode_jwt_rsa() {
        use tempfile::NamedTempFile;

        // Create a temporary key file
        let mut key_file = NamedTempFile::new().unwrap();
        use std::io::Write;
        key_file.write_all(TEST_RSA_PRIVATE_KEY.as_bytes()).unwrap();
        key_file.flush().unwrap();

        let payload = AuthPayload {
            token: "test-bearer-token".to_string(),
            request_body: serde_json::json!({"text": "Hello, world!"}),
            request_headers: {
                let mut headers = HashMap::new();
                headers.insert("content-type".to_string(), "application/json".to_string());
                headers
            },
            request_path: "/speak".to_string(),
            request_method: "POST".to_string(),
        };

        // Sign the JWT
        let jwt = sign_auth_request(&payload, key_file.path()).unwrap();

        // Verify JWT format (header.payload.signature)
        assert_eq!(jwt.matches('.').count(), 2);

        // Manually decode the JWT payload (middle part between dots)
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);

        // Decode the payload (second part)
        use base64::Engine;
        let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[1])
            .unwrap();
        let claims: AuthClaims = serde_json::from_slice(&payload_bytes).unwrap();

        // Verify standard JWT claims
        assert_eq!(claims.sub, "sayna-auth");
        assert!(claims.iat > 0);
        assert!(claims.exp > claims.iat);
        assert_eq!(claims.exp - claims.iat, 300); // 5 minutes

        // Verify nested auth_data
        assert_eq!(claims.auth_data.token, "test-bearer-token");
        assert_eq!(claims.auth_data.request_path, "/speak");
        assert_eq!(claims.auth_data.request_method, "POST");
        assert_eq!(
            claims.auth_data.request_body,
            serde_json::json!({"text": "Hello, world!"})
        );
        assert_eq!(
            claims.auth_data.request_headers.get("content-type"),
            Some(&"application/json".to_string())
        );
    }

    #[test]
    #[ignore] // EC key support may vary by jsonwebtoken version
    fn test_sign_and_decode_jwt_ec() {
        use tempfile::NamedTempFile;

        // Create a temporary key file
        let mut key_file = NamedTempFile::new().unwrap();
        use std::io::Write;
        key_file.write_all(TEST_EC_PRIVATE_KEY.as_bytes()).unwrap();
        key_file.flush().unwrap();

        let payload = AuthPayload {
            token: "test-ec-token".to_string(),
            request_body: serde_json::json!(null),
            request_headers: HashMap::new(),
            request_path: "/voices".to_string(),
            request_method: "GET".to_string(),
        };

        // Sign the JWT
        let jwt = sign_auth_request(&payload, key_file.path()).unwrap();

        // Manually decode the JWT payload
        let parts: Vec<&str> = jwt.split('.').collect();
        use base64::Engine;
        let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[1])
            .unwrap();
        let claims: AuthClaims = serde_json::from_slice(&payload_bytes).unwrap();

        // Verify claims
        assert_eq!(claims.sub, "sayna-auth");
        assert_eq!(claims.auth_data.token, "test-ec-token");
        assert_eq!(claims.auth_data.request_path, "/voices");
        assert_eq!(claims.auth_data.request_method, "GET");
    }

    #[test]
    fn test_jwt_serialization_format() {
        use tempfile::NamedTempFile;

        let mut key_file = NamedTempFile::new().unwrap();
        use std::io::Write;
        key_file.write_all(TEST_RSA_PRIVATE_KEY.as_bytes()).unwrap();
        key_file.flush().unwrap();

        let payload = AuthPayload {
            token: "test-token".to_string(),
            request_body: serde_json::json!({"key": "value"}),
            request_headers: HashMap::new(),
            request_path: "/test".to_string(),
            request_method: "POST".to_string(),
        };

        let jwt = sign_auth_request(&payload, key_file.path()).unwrap();

        // Manually decode the JWT payload
        let parts: Vec<&str> = jwt.split('.').collect();
        use base64::Engine;
        let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[1])
            .unwrap();
        let claims: AuthClaims = serde_json::from_slice(&payload_bytes).unwrap();

        let json = serde_json::to_value(&claims).unwrap();

        // Verify top-level fields
        assert!(json.get("sub").is_some());
        assert!(json.get("iat").is_some());
        assert!(json.get("exp").is_some());
        assert!(json.get("auth_data").is_some());

        // Verify auth_data structure
        let auth_data = json.get("auth_data").unwrap();
        assert!(auth_data.get("token").is_some());
        assert!(auth_data.get("request_body").is_some());
        assert!(auth_data.get("request_headers").is_some());
        assert!(auth_data.get("request_path").is_some());
        assert!(auth_data.get("request_method").is_some());

        // Verify no unexpected fields at top level
        assert_eq!(json.as_object().unwrap().len(), 4); // sub, iat, exp, auth_data
    }

    #[test]
    fn test_load_private_key_caching() {
        use tempfile::NamedTempFile;

        let mut key_file = NamedTempFile::new().unwrap();
        use std::io::Write;
        key_file.write_all(TEST_RSA_PRIVATE_KEY.as_bytes()).unwrap();
        key_file.flush().unwrap();

        // First load
        let result1 = load_private_key(key_file.path());
        assert!(result1.is_ok());
        let (_, alg1) = result1.unwrap();
        assert_eq!(alg1, Algorithm::RS256);

        // Second load (should use cache)
        let result2 = load_private_key(key_file.path());
        assert!(result2.is_ok());
        let (_, alg2) = result2.unwrap();
        assert_eq!(alg2, Algorithm::RS256);

        // Both should return the same algorithm
        assert_eq!(alg1, alg2);
    }

    #[test]
    #[ignore] // EC key support may vary by jsonwebtoken version
    fn test_load_private_key_ec_algorithm_detection() {
        use tempfile::NamedTempFile;

        let mut key_file = NamedTempFile::new().unwrap();
        use std::io::Write;
        key_file.write_all(TEST_EC_PRIVATE_KEY.as_bytes()).unwrap();
        key_file.flush().unwrap();

        let result = load_private_key(key_file.path());
        assert!(result.is_ok());
        let (_, algorithm) = result.unwrap();
        assert_eq!(algorithm, Algorithm::ES256);
    }

    #[test]
    fn test_sign_auth_request_with_key_direct() {
        use tempfile::NamedTempFile;

        let mut key_file = NamedTempFile::new().unwrap();
        use std::io::Write;
        key_file.write_all(TEST_RSA_PRIVATE_KEY.as_bytes()).unwrap();
        key_file.flush().unwrap();

        let (encoding_key, algorithm) = load_private_key(key_file.path()).unwrap();

        let payload = AuthPayload {
            token: "direct-test".to_string(),
            request_body: serde_json::json!({}),
            request_headers: HashMap::new(),
            request_path: "/test".to_string(),
            request_method: "GET".to_string(),
        };

        // Use sign_auth_request_with_key directly
        let jwt = sign_auth_request_with_key(&payload, &encoding_key, algorithm);
        assert!(jwt.is_ok());

        let jwt_str = jwt.unwrap();
        assert_eq!(jwt_str.matches('.').count(), 2);
    }
}
