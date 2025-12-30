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

    /// Normalizes a room name by prefixing it with the authenticated client's ID.
    ///
    /// This ensures room isolation between different authenticated clients.
    /// If the room name already contains the auth prefix, it is returned unchanged
    /// to prevent duplicate prefixes.
    ///
    /// # Arguments
    /// * `room_name` - The room name to normalize
    ///
    /// # Returns
    /// The normalized room name with auth prefix, or the original if:
    /// - No auth.id is present (unauthenticated request)
    /// - Room name already has the correct prefix
    ///
    /// # Examples
    /// ```
    /// use sayna::auth::Auth;
    ///
    /// let auth = Auth::new("project1");
    /// assert_eq!(auth.normalize_room_name("my-room"), "project1_my-room");
    /// assert_eq!(auth.normalize_room_name("project1_my-room"), "project1_my-room"); // No duplicate
    ///
    /// let empty_auth = Auth::empty();
    /// assert_eq!(empty_auth.normalize_room_name("my-room"), "my-room"); // Unchanged
    /// ```
    pub fn normalize_room_name(&self, room_name: &str) -> String {
        match &self.id {
            Some(auth_id) if !auth_id.is_empty() => {
                let prefix = format!("{}_", auth_id);
                if room_name.starts_with(&prefix) {
                    // Already prefixed, return unchanged
                    room_name.to_string()
                } else {
                    // Add prefix
                    format!("{}_{}", auth_id, room_name)
                }
            }
            // No auth.id present or empty, return unchanged
            _ => room_name.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_room_name_basic_prefix() {
        let auth = Auth::new("project1");
        assert_eq!(auth.normalize_room_name("my-room"), "project1_my-room");
    }

    #[test]
    fn test_normalize_room_name_no_duplicate_prefix() {
        let auth = Auth::new("project1");
        assert_eq!(
            auth.normalize_room_name("project1_my-room"),
            "project1_my-room"
        );
    }

    #[test]
    fn test_normalize_room_name_empty_auth() {
        let auth = Auth::empty();
        assert_eq!(auth.normalize_room_name("my-room"), "my-room");
    }

    #[test]
    fn test_normalize_room_name_empty_auth_id() {
        let auth = Auth {
            id: Some("".to_string()),
        };
        assert_eq!(auth.normalize_room_name("my-room"), "my-room");
    }

    #[test]
    fn test_normalize_room_name_empty_room() {
        let auth = Auth::new("project1");
        assert_eq!(auth.normalize_room_name(""), "project1_");
    }

    #[test]
    fn test_normalize_room_name_with_underscore_in_room() {
        let auth = Auth::new("auth1");
        // Room name happens to contain underscore but doesn't start with auth prefix
        assert_eq!(auth.normalize_room_name("my_room"), "auth1_my_room");
    }

    #[test]
    fn test_normalize_room_name_partial_prefix_match() {
        let auth = Auth::new("project1");
        // Room starts with auth_id but without underscore separator
        assert_eq!(
            auth.normalize_room_name("project1room"),
            "project1_project1room"
        );
    }

    #[test]
    fn test_normalize_room_name_case_sensitive() {
        let auth = Auth::new("Project1");
        // Prefix match is case-sensitive
        assert_eq!(
            auth.normalize_room_name("project1_my-room"),
            "Project1_project1_my-room"
        );
        assert_eq!(
            auth.normalize_room_name("Project1_my-room"),
            "Project1_my-room"
        );
    }

    #[test]
    fn test_normalize_room_name_special_chars_in_auth_id() {
        let auth = Auth::new("proj-123");
        assert_eq!(auth.normalize_room_name("room"), "proj-123_room");
        assert_eq!(auth.normalize_room_name("proj-123_room"), "proj-123_room");
    }

    #[test]
    fn test_normalize_room_name_unicode() {
        let auth = Auth::new("测试");
        assert_eq!(auth.normalize_room_name("room"), "测试_room");
        assert_eq!(auth.normalize_room_name("测试_room"), "测试_room");
    }
}
