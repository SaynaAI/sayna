//! Google Cloud API error types.
//!
//! This module provides generic error types for Google Cloud API operations
//! that can be used across different Google services (STT, TTS, etc.).

/// Error types for Google Cloud API operations.
#[derive(Debug, thiserror::Error)]
pub enum GoogleError {
    /// Authentication failed (invalid credentials, expired token, etc.)
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    /// Configuration error (invalid project ID, missing credentials, etc.)
    #[error("Configuration error: {0}")]
    ConfigurationError(String),

    /// Network or connection error
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    /// Network-level error (timeout, DNS failure, etc.)
    #[error("Network error: {0}")]
    NetworkError(String),

    /// Error from the Google Cloud API
    #[error("API error: {0}")]
    ApiError(String),

    /// gRPC-specific error with status code
    #[error("gRPC error ({code}): {message}")]
    GrpcError { code: String, message: String },
}

impl GoogleError {
    /// Creates a gRPC error from a tonic Status.
    pub fn from_grpc_status(status: &tonic::Status) -> Self {
        Self::GrpcError {
            code: format!("{:?}", status.code()),
            message: status.message().to_string(),
        }
    }

    /// Categorizes a gRPC error based on the status code.
    pub fn categorize_grpc_error(status: tonic::Status) -> Self {
        match status.code() {
            tonic::Code::Unauthenticated | tonic::Code::PermissionDenied => {
                Self::AuthenticationFailed(status.message().to_string())
            }
            tonic::Code::InvalidArgument => Self::ConfigurationError(status.message().to_string()),
            tonic::Code::Unavailable | tonic::Code::DeadlineExceeded => {
                Self::NetworkError(status.message().to_string())
            }
            _ => Self::from_grpc_status(&status),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let auth_err = GoogleError::AuthenticationFailed("Invalid token".to_string());
        assert_eq!(auth_err.to_string(), "Authentication failed: Invalid token");

        let config_err = GoogleError::ConfigurationError("Missing project ID".to_string());
        assert_eq!(
            config_err.to_string(),
            "Configuration error: Missing project ID"
        );

        let conn_err = GoogleError::ConnectionFailed("Connection refused".to_string());
        assert_eq!(
            conn_err.to_string(),
            "Connection failed: Connection refused"
        );

        let grpc_err = GoogleError::GrpcError {
            code: "Unavailable".to_string(),
            message: "Service unavailable".to_string(),
        };
        assert_eq!(
            grpc_err.to_string(),
            "gRPC error (Unavailable): Service unavailable"
        );
    }

    #[test]
    fn test_from_grpc_status() {
        let status = tonic::Status::unavailable("Service unavailable");
        let err = GoogleError::from_grpc_status(&status);

        match err {
            GoogleError::GrpcError { code, message } => {
                assert_eq!(code, "Unavailable");
                assert_eq!(message, "Service unavailable");
            }
            _ => panic!("Expected GrpcError"),
        }
    }

    #[test]
    fn test_categorize_grpc_error_unauthenticated() {
        let status = tonic::Status::unauthenticated("Invalid credentials");
        let err = GoogleError::categorize_grpc_error(status);

        match err {
            GoogleError::AuthenticationFailed(msg) => {
                assert_eq!(msg, "Invalid credentials");
            }
            _ => panic!("Expected AuthenticationFailed"),
        }
    }

    #[test]
    fn test_categorize_grpc_error_invalid_argument() {
        let status = tonic::Status::invalid_argument("Bad request");
        let err = GoogleError::categorize_grpc_error(status);

        match err {
            GoogleError::ConfigurationError(msg) => {
                assert_eq!(msg, "Bad request");
            }
            _ => panic!("Expected ConfigurationError"),
        }
    }

    #[test]
    fn test_categorize_grpc_error_unavailable() {
        let status = tonic::Status::unavailable("Try again later");
        let err = GoogleError::categorize_grpc_error(status);

        match err {
            GoogleError::NetworkError(msg) => {
                assert_eq!(msg, "Try again later");
            }
            _ => panic!("Expected NetworkError"),
        }
    }
}
