/// Authenticated client information
///
/// Inserted into request extensions by the auth middleware after successful
/// validation. Handlers can extract this via `Extension<Auth>` to access
/// the authenticated client's identity.
///
/// This struct is designed to be extensible - new fields can be added
/// as the auth service evolves.
///
/// # Example
/// ```rust,ignore
/// use axum::Extension;
/// use sayna::auth::Auth;
///
/// async fn my_handler(
///     Extension(auth): Extension<Auth>,
/// ) -> impl IntoResponse {
///     if let Some(id) = &auth.id {
///         tracing::info!("Request from client: {}", id);
///     }
///     // ...
/// }
/// ```
#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct Auth {
    /// The authenticated client/project identifier (e.g., "project1", "client-a")
    #[serde(default)]
    pub id: Option<String>,
    // Future fields can be added here, e.g.:
    // pub org_id: Option<String>,
    // pub permissions: Vec<String>,
    // pub metadata: Option<serde_json::Value>,
}

impl Auth {
    /// Create a new Auth with the given id
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: Some(id.into()),
        }
    }

    /// Create an empty Auth (no id)
    pub fn empty() -> Self {
        Self::default()
    }
}
