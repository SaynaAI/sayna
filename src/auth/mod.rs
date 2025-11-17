pub mod client;
pub mod jwt;

// Re-export commonly used items
pub use client::AuthClient;
pub use jwt::{
    AuthClaims, AuthPayload, detect_algorithm, filter_headers, load_private_key, sign_auth_request,
    sign_auth_request_with_key,
};
