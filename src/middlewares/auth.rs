use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use axum::{
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use pbkdf2::pbkdf2_hmac;
use sha2::Sha256;
use std::sync::Arc;

use crate::state::AppState;

/// Constants matching the Node.js encryption scheme
const ALGO_TAG_LEN: usize = 16; // GCM auth tag length
const SALT_LEN: usize = 16; // Salt length in bytes
const IV_LEN: usize = 12; // IV length for GCM
const KEY_LEN: usize = 32; // 256 bits for AES-256
const PBKDF2_ITERS: u32 = 100_000; // PBKDF2 iterations
const EXPECTED_VERSION: u8 = 1; // Token version

/// Error type for authentication failures
#[derive(Debug)]
pub enum AuthError {
    MissingAuthHeader,
    InvalidAuthFormat,
    InvalidToken(String),
    DecryptionFailed(String),
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let message = match self {
            AuthError::MissingAuthHeader => "Missing Authorization header".to_string(),
            AuthError::InvalidAuthFormat => {
                "Invalid Authorization format. Expected: Bearer sayna_<token>".to_string()
            }
            AuthError::InvalidToken(msg) => msg,
            AuthError::DecryptionFailed(msg) => msg,
        };

        (StatusCode::UNAUTHORIZED, message).into_response()
    }
}

/// Extract and validate the Bearer token from Authorization header
///
/// Expected format: "Authorization: Bearer sayna_<base64_encrypted_project_id>"
fn extract_token(headers: &HeaderMap) -> Result<String, AuthError> {
    let auth_header = headers
        .get("authorization")
        .ok_or(AuthError::MissingAuthHeader)?
        .to_str()
        .map_err(|_| AuthError::InvalidAuthFormat)?;

    // Expected format: "Bearer sayna_<token>"
    if !auth_header.starts_with("Bearer ") {
        return Err(AuthError::InvalidAuthFormat);
    }

    let token_part = auth_header
        .strip_prefix("Bearer ")
        .ok_or(AuthError::InvalidAuthFormat)?;

    // Check for "sayna_" prefix
    if !token_part.starts_with("sayna_") {
        return Err(AuthError::InvalidAuthFormat);
    }

    let encrypted_token = token_part
        .strip_prefix("sayna_")
        .ok_or(AuthError::InvalidAuthFormat)?;

    Ok(encrypted_token.to_string())
}

/// Decrypt the API key token to recover the project ID
///
/// This function implements the same decryption logic as the Node.js code:
/// - Token format: version(1) || salt(16) || iv(12) || ciphertext || tag(16)
/// - Uses PBKDF2 with 100,000 iterations and SHA-256
/// - AES-256-GCM for encryption
///
/// # Arguments
/// * `token_base64` - Base64-encoded token produced by the Node.js createApiKey function
/// * `secret` - Shared secret key for decryption
///
/// # Returns
/// * `Result<String, AuthError>` - Decrypted project ID or error
fn decrypt_api_key(token_base64: &str, secret: &str) -> Result<String, AuthError> {
    // Decode base64 token
    let token_buf = BASE64
        .decode(token_base64)
        .map_err(|e| AuthError::InvalidToken(format!("Invalid base64: {e}")))?;

    // Minimum length check: version(1) + salt(16) + iv(12) + tag(16) = 45 bytes minimum
    let min_len = 1 + SALT_LEN + IV_LEN + ALGO_TAG_LEN;
    if token_buf.len() < min_len {
        return Err(AuthError::InvalidToken(format!(
            "Token too short: expected at least {min_len} bytes, got {}",
            token_buf.len()
        )));
    }

    // Parse token structure
    let mut offset = 0;

    // Version (1 byte)
    let version = token_buf[offset];
    offset += 1;
    if version != EXPECTED_VERSION {
        return Err(AuthError::InvalidToken(format!(
            "Unsupported token version: {version}"
        )));
    }

    // Salt (16 bytes)
    let salt = &token_buf[offset..offset + SALT_LEN];
    offset += SALT_LEN;

    // IV (12 bytes)
    let iv = &token_buf[offset..offset + IV_LEN];
    offset += IV_LEN;

    // Tag is the last 16 bytes
    let tag_start = token_buf.len() - ALGO_TAG_LEN;
    let tag = &token_buf[tag_start..];

    // Ciphertext is everything between IV and tag
    let ciphertext = &token_buf[offset..tag_start];

    // Derive key using PBKDF2
    let mut key = [0u8; KEY_LEN];
    pbkdf2_hmac::<Sha256>(secret.as_bytes(), salt, PBKDF2_ITERS, &mut key);

    // Decrypt using AES-256-GCM
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| AuthError::DecryptionFailed(format!("Failed to create cipher: {e}")))?;

    // Combine ciphertext and tag for AES-GCM
    let mut combined = Vec::with_capacity(ciphertext.len() + tag.len());
    combined.extend_from_slice(ciphertext);
    combined.extend_from_slice(tag);

    let nonce = Nonce::from_slice(iv);

    let plaintext = cipher.decrypt(nonce, combined.as_ref()).map_err(|e| {
        AuthError::DecryptionFailed(format!(
            "Decryption failed (invalid key or corrupted data): {e}"
        ))
    })?;

    // Convert plaintext to UTF-8 string
    String::from_utf8(plaintext)
        .map_err(|e| AuthError::DecryptionFailed(format!("Invalid UTF-8 in decrypted data: {e}")))
}

/// Axum middleware for authentication
///
/// This middleware:
/// 1. Checks if AUTH_DECRYPTION_KEY is configured
/// 2. If empty, allows all requests through (auth disabled)
/// 3. If set, validates and decrypts the Bearer token
/// 4. Returns 401 if authentication fails
///
/// # Usage
/// ```rust,ignore
/// use axum::Router;
/// use axum::routing::get;
/// use axum::middleware;
/// use std::sync::Arc;
/// use sayna::middlewares::auth::auth_middleware;
/// use sayna::state::AppState;
///
/// async fn example(app_state: Arc<AppState>) {
///     let app = Router::new()
///         .route("/protected", get(|| async { "Protected route" }))
///         .layer(middleware::from_fn_with_state(
///             app_state.clone(),
///             auth_middleware
///         ))
///         .with_state(app_state);
/// }
/// ```
pub async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    mut request: Request,
    next: Next,
) -> Result<Response, AuthError> {
    let decryption_key = &state.config.auth_decryption_key;

    // If decryption key is empty, auth is disabled - allow all requests
    if decryption_key.is_empty() {
        return Ok(next.run(request).await);
    }

    // Extract and validate token from request headers
    let headers = request.headers().clone();
    let encrypted_token = extract_token(&headers)?;

    // Decrypt token to get project_id
    let _project_id = decrypt_api_key(&encrypted_token, decryption_key)?;

    // Store the project_id in request extensions if needed
    // for downstream handlers to use:
    request.extensions_mut().insert(_project_id.clone());

    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_token_valid() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            "Bearer sayna_dGVzdF90b2tlbg==".parse().unwrap(),
        );

        let result = extract_token(&headers);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "dGVzdF90b2tlbg==");
    }

    #[test]
    fn test_extract_token_missing_header() {
        let headers = HeaderMap::new();
        let result = extract_token(&headers);
        assert!(matches!(result, Err(AuthError::MissingAuthHeader)));
    }

    #[test]
    fn test_extract_token_invalid_format_no_bearer() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "sayna_token".parse().unwrap());

        let result = extract_token(&headers);
        assert!(matches!(result, Err(AuthError::InvalidAuthFormat)));
    }

    #[test]
    fn test_extract_token_invalid_format_no_sayna_prefix() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer token123".parse().unwrap());

        let result = extract_token(&headers);
        assert!(matches!(result, Err(AuthError::InvalidAuthFormat)));
    }

    #[test]
    fn test_decrypt_api_key_invalid_base64() {
        let result = decrypt_api_key("not-valid-base64!!!", "secret");
        assert!(matches!(result, Err(AuthError::InvalidToken(_))));
    }

    #[test]
    fn test_decrypt_api_key_too_short() {
        // Create a token that's too short (less than 45 bytes)
        let short_token = BASE64.encode(b"short");
        let result = decrypt_api_key(&short_token, "secret");
        assert!(matches!(result, Err(AuthError::InvalidToken(_))));
    }

    #[test]
    fn test_decrypt_api_key_invalid_version() {
        // Create a token with wrong version (version 2 instead of 1)
        let mut token = vec![2u8]; // wrong version
        token.extend_from_slice(&[0u8; SALT_LEN + IV_LEN + ALGO_TAG_LEN]);
        let token_base64 = BASE64.encode(&token);

        let result = decrypt_api_key(&token_base64, "secret");
        assert!(matches!(result, Err(AuthError::InvalidToken(_))));
    }

    // Note: Testing successful decryption requires generating a valid token
    // using the Node.js encryption code. This is better done in integration tests.
}
