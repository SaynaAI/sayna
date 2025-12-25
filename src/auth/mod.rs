pub mod api_secret;
pub mod client;
pub mod context;
pub mod jwt;

// Re-export commonly used items
pub use api_secret::match_api_secret_id;
pub use client::AuthClient;
pub use context::Auth;
pub use jwt::{
    AuthClaims, AuthPayload, detect_algorithm, filter_headers, load_private_key, sign_auth_request,
    sign_auth_request_with_key,
};
