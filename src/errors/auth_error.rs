use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

/// Error codes for structured error responses
pub mod error_codes {
    pub const MISSING_AUTH_HEADER: &str = "missing_auth_header";
    pub const INVALID_AUTH_HEADER: &str = "invalid_auth_header";
    pub const AUTH_SERVICE_UNAVAILABLE: &str = "auth_service_unavailable";
    pub const AUTH_SERVICE_ERROR: &str = "auth_service_error";
    pub const UNAUTHORIZED: &str = "unauthorized";
    pub const JWT_SIGNING_ERROR: &str = "jwt_signing_error";
    pub const CONFIG_ERROR: &str = "config_error";
}

/// Authentication error types
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// Authorization header is missing from request
    #[error("Missing Authorization header")]
    MissingAuthHeader,

    /// Authorization header format is invalid (not "Bearer {token}")
    #[error("Invalid Authorization header format")]
    InvalidAuthHeader,

    /// Auth service is unavailable or unreachable
    #[error("Auth service unavailable: {0}")]
    AuthServiceUnavailable(String),

    /// Auth service returned an error response
    #[error("Auth service error ({0}): {1}")]
    AuthServiceError(StatusCode, String),

    /// Token validation failed (unauthorized)
    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    /// JWT signing operation failed
    #[error("JWT signing error: {0}")]
    JwtSigningError(String),

    /// Configuration error (missing required auth config)
    #[error("Auth configuration error: {0}")]
    ConfigError(String),

    /// HTTP request error
    #[error("HTTP request error: {0}")]
    HttpError(#[from] reqwest::Error),

    /// JWT encoding error
    #[error("JWT encoding error: {0}")]
    JwtError(#[from] jsonwebtoken::errors::Error),
}

impl AuthError {
    /// Get the error code for structured error responses
    pub fn error_code(&self) -> &'static str {
        match self {
            AuthError::MissingAuthHeader => error_codes::MISSING_AUTH_HEADER,
            AuthError::InvalidAuthHeader => error_codes::INVALID_AUTH_HEADER,
            AuthError::AuthServiceUnavailable(_) => error_codes::AUTH_SERVICE_UNAVAILABLE,
            AuthError::AuthServiceError(_, _) => error_codes::AUTH_SERVICE_ERROR,
            AuthError::Unauthorized(_) => error_codes::UNAUTHORIZED,
            AuthError::JwtSigningError(_) => error_codes::JWT_SIGNING_ERROR,
            AuthError::ConfigError(_) => error_codes::CONFIG_ERROR,
            AuthError::HttpError(_) => error_codes::AUTH_SERVICE_UNAVAILABLE,
            AuthError::JwtError(_) => error_codes::JWT_SIGNING_ERROR,
        }
    }

    /// Get the HTTP status code for this error
    pub fn status_code(&self) -> StatusCode {
        match self {
            AuthError::MissingAuthHeader | AuthError::InvalidAuthHeader => StatusCode::UNAUTHORIZED,
            AuthError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            AuthError::AuthServiceUnavailable(_) | AuthError::HttpError(_) => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            AuthError::AuthServiceError(status, _) => {
                // If the auth service returns 4xx, map to 401
                // If it returns 5xx, map to 502 (bad gateway)
                if status.is_client_error() {
                    StatusCode::UNAUTHORIZED
                } else {
                    StatusCode::BAD_GATEWAY
                }
            }
            AuthError::JwtSigningError(_) | AuthError::ConfigError(_) | AuthError::JwtError(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }

    /// Log the error at the appropriate level
    pub fn log(&self) {
        match self {
            // Debug level for expected auth failures (missing/invalid headers)
            AuthError::MissingAuthHeader | AuthError::InvalidAuthHeader => {
                tracing::debug!("{}", self);
            }
            // Warn level for auth service issues and unauthorized requests
            AuthError::Unauthorized(msg) => {
                tracing::warn!("Unauthorized: {}", msg);
            }
            AuthError::AuthServiceError(code, msg) => {
                tracing::warn!("Auth service error ({}): {}", code, msg);
            }
            // Error level for system issues
            AuthError::AuthServiceUnavailable(msg) => {
                tracing::error!("Auth service unavailable: {}", msg);
            }
            AuthError::JwtSigningError(msg) => {
                tracing::error!("JWT signing error: {}", msg);
            }
            AuthError::ConfigError(msg) => {
                tracing::error!("Auth configuration error: {}", msg);
            }
            AuthError::HttpError(err) => {
                tracing::error!("Auth HTTP error: {}", err);
            }
            AuthError::JwtError(err) => {
                tracing::error!("JWT error: {}", err);
            }
        }
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        // Log the error
        self.log();

        let status = self.status_code();
        let error_code = self.error_code();
        let error_message = self.to_string();

        // Response format: {"error": "error_code", "message": "human readable message"}
        let body = Json(json!({
            "error": error_code,
            "message": error_message
        }));

        (status, body).into_response()
    }
}

// Result type alias for convenience
pub type AuthResult<T> = Result<T, AuthError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_codes() {
        assert_eq!(
            AuthError::MissingAuthHeader.error_code(),
            error_codes::MISSING_AUTH_HEADER
        );
        assert_eq!(
            AuthError::InvalidAuthHeader.error_code(),
            error_codes::INVALID_AUTH_HEADER
        );
        assert_eq!(
            AuthError::Unauthorized("test".to_string()).error_code(),
            error_codes::UNAUTHORIZED
        );
    }

    #[test]
    fn test_status_codes() {
        assert_eq!(
            AuthError::MissingAuthHeader.status_code(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            AuthError::InvalidAuthHeader.status_code(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            AuthError::Unauthorized("test".to_string()).status_code(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            AuthError::AuthServiceUnavailable("test".to_string()).status_code(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            AuthError::ConfigError("test".to_string()).status_code(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn test_auth_service_error_status_mapping() {
        // 401 from auth service -> 401
        assert_eq!(
            AuthError::AuthServiceError(StatusCode::UNAUTHORIZED, "test".to_string()).status_code(),
            StatusCode::UNAUTHORIZED
        );

        // 403 from auth service -> 401
        assert_eq!(
            AuthError::AuthServiceError(StatusCode::FORBIDDEN, "test".to_string()).status_code(),
            StatusCode::UNAUTHORIZED
        );

        // 500 from auth service -> 502
        assert_eq!(
            AuthError::AuthServiceError(StatusCode::INTERNAL_SERVER_ERROR, "test".to_string())
                .status_code(),
            StatusCode::BAD_GATEWAY
        );

        // 503 from auth service -> 502
        assert_eq!(
            AuthError::AuthServiceError(StatusCode::SERVICE_UNAVAILABLE, "test".to_string())
                .status_code(),
            StatusCode::BAD_GATEWAY
        );
    }

    #[test]
    fn test_display() {
        assert_eq!(
            AuthError::MissingAuthHeader.to_string(),
            "Missing Authorization header"
        );
        assert_eq!(
            AuthError::InvalidAuthHeader.to_string(),
            "Invalid Authorization header format"
        );
        assert_eq!(
            AuthError::Unauthorized("invalid token".to_string()).to_string(),
            "Unauthorized: invalid token"
        );
    }

    #[test]
    fn test_into_response_missing_auth_header() {
        use http_body_util::BodyExt;

        let error = AuthError::MissingAuthHeader;
        let response = error.into_response();

        // Check status code
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        // Check body structure
        let body = response.into_body();
        let body_bytes = tokio_test::block_on(async { body.collect().await.unwrap().to_bytes() });
        let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

        assert_eq!(body_json["error"], "missing_auth_header");
        assert_eq!(body_json["message"], "Missing Authorization header");
        assert!(body_json.get("status").is_none()); // status field should not be present
        assert!(body_json.get("error_code").is_none()); // error_code field should not be present
    }

    #[test]
    fn test_into_response_invalid_auth_header() {
        use http_body_util::BodyExt;

        let error = AuthError::InvalidAuthHeader;
        let response = error.into_response();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let body_bytes = tokio_test::block_on(async {
            response.into_body().collect().await.unwrap().to_bytes()
        });
        let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

        assert_eq!(body_json["error"], "invalid_auth_header");
        assert_eq!(body_json["message"], "Invalid Authorization header format");
    }

    #[test]
    fn test_into_response_unauthorized() {
        use http_body_util::BodyExt;

        let error = AuthError::Unauthorized("Invalid token signature".to_string());
        let response = error.into_response();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let body_bytes = tokio_test::block_on(async {
            response.into_body().collect().await.unwrap().to_bytes()
        });
        let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

        assert_eq!(body_json["error"], "unauthorized");
        assert_eq!(
            body_json["message"],
            "Unauthorized: Invalid token signature"
        );
    }

    #[test]
    fn test_into_response_auth_service_error_4xx() {
        use http_body_util::BodyExt;

        let error =
            AuthError::AuthServiceError(StatusCode::FORBIDDEN, "Forbidden resource".to_string());
        let response = error.into_response();

        // 4xx from auth service should map to 401
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let body_bytes = tokio_test::block_on(async {
            response.into_body().collect().await.unwrap().to_bytes()
        });
        let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

        assert_eq!(body_json["error"], "auth_service_error");
        assert_eq!(
            body_json["message"],
            "Auth service error (403 Forbidden): Forbidden resource"
        );
    }

    #[test]
    fn test_into_response_auth_service_error_5xx() {
        use http_body_util::BodyExt;

        let error = AuthError::AuthServiceError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal error".to_string(),
        );
        let response = error.into_response();

        // 5xx from auth service should map to 502 Bad Gateway
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

        let body_bytes = tokio_test::block_on(async {
            response.into_body().collect().await.unwrap().to_bytes()
        });
        let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

        assert_eq!(body_json["error"], "auth_service_error");
        assert!(
            body_json["message"]
                .as_str()
                .unwrap()
                .contains("500 Internal Server Error")
        );
    }

    #[test]
    fn test_into_response_config_error() {
        use http_body_util::BodyExt;

        let error = AuthError::ConfigError("Missing AUTH_SERVICE_URL".to_string());
        let response = error.into_response();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let body_bytes = tokio_test::block_on(async {
            response.into_body().collect().await.unwrap().to_bytes()
        });
        let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

        assert_eq!(body_json["error"], "config_error");
        assert_eq!(
            body_json["message"],
            "Auth configuration error: Missing AUTH_SERVICE_URL"
        );
    }

    #[test]
    fn test_into_response_service_unavailable() {
        use http_body_util::BodyExt;

        let error = AuthError::AuthServiceUnavailable("Connection timeout".to_string());
        let response = error.into_response();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

        let body_bytes = tokio_test::block_on(async {
            response.into_body().collect().await.unwrap().to_bytes()
        });
        let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

        assert_eq!(body_json["error"], "auth_service_unavailable");
        assert_eq!(
            body_json["message"],
            "Auth service unavailable: Connection timeout"
        );
    }
}
