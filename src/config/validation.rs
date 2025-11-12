use std::path::PathBuf;

/// Validate JWT authentication configuration
///
/// Ensures that if either AUTH_SERVICE_URL or AUTH_SIGNING_KEY_PATH is provided,
/// both must be present and the key file must exist.
pub fn validate_jwt_auth(
    auth_service_url: &Option<String>,
    auth_signing_key_path: &Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    // If either is set, both must be set
    if auth_service_url.is_some() || auth_signing_key_path.is_some() {
        if auth_service_url.is_none() {
            return Err("AUTH_SERVICE_URL is required when AUTH_SIGNING_KEY_PATH is set".into());
        }
        if auth_signing_key_path.is_none() {
            return Err("AUTH_SIGNING_KEY_PATH is required when AUTH_SERVICE_URL is set".into());
        }

        // Check if the signing key file exists
        if let Some(key_path) = auth_signing_key_path {
            if !key_path.exists() {
                return Err(format!(
                    "AUTH_SIGNING_KEY_PATH file does not exist: {}",
                    key_path.display()
                )
                .into());
            }
        }
    }

    Ok(())
}

/// Validate that when auth is required, at least one auth method is configured
///
/// Checks that either JWT auth (service URL + signing key) or API secret is present
/// when authentication is required.
pub fn validate_auth_required(
    auth_required: bool,
    auth_service_url: &Option<String>,
    auth_signing_key_path: &Option<PathBuf>,
    auth_api_secret: &Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    if !auth_required {
        return Ok(());
    }

    let has_jwt_auth = auth_service_url.is_some() && auth_signing_key_path.is_some();
    let has_api_secret = auth_api_secret.is_some();

    if !has_jwt_auth && !has_api_secret {
        return Err(
            "When AUTH_REQUIRED=true, either (AUTH_SERVICE_URL + AUTH_SIGNING_KEY_PATH) or AUTH_API_SECRET must be configured".into()
        );
    }

    Ok(())
}
